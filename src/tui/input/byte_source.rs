//! Byte-level terminal input handles that separate bounded reads from the idle wait.
//!
//! Decoding one input event runs inside the editor's event loop, which cannot
//! repaint, resize, or notice a termination signal while a decode is in flight.
//! Every read a decoder can reach therefore carries a deadline: those live on
//! [`BoundedInput`], which is the only handle the decoders receive. The one read
//! that may wait forever stays on [`InputSource`], whose private stdin handle is
//! unreachable from the decoding module.

use super::super::unsafe_io;
use crate::unsafe_io::poll_fd;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Stdin, stdin};
use std::time::{Duration, Instant};

thread_local! {
    /// Store same-thread lookahead bytes that were read while decoding one key
    /// sequence but belong to the next input event.
    ///
    /// Escape-sequence parsing and UTF-8 fallback sometimes need to "unread" one
    /// byte so the next read can consume it. The application loop reads terminal
    /// input on one thread (`src/app.rs`), so keeping this queue thread-local
    /// preserves the intended behavior without sharing parser state across
    /// unrelated threads or tests.
    static PENDING_BYTES: RefCell<VecDeque<u8>> = const { RefCell::new(VecDeque::new()) };
}

/// Return whether the current thread already has deferred lookahead bytes.
///
/// Returns `true` when a pushed-back byte is waiting to be consumed ahead of any
/// newly available terminal input, and `false` when the queue is empty.
pub(super) fn pending_queue_has_bytes() -> bool {
    PENDING_BYTES.with(|queue| !queue.borrow().is_empty())
}

/// Push one lookahead byte back so the next read consumes it first.
///
/// This keeps multi-byte sequence parsing and later top-level input reads in
/// sync when the parser had to hand one byte back to itself.
pub(super) fn push_pending_byte(byte: u8) {
    PENDING_BYTES.with(|queue| queue.borrow_mut().push_back(byte));
}

/// Pop one previously deferred lookahead byte for the current thread.
fn pop_pending_byte() -> Option<u8> {
    PENDING_BYTES.with(|queue| queue.borrow_mut().pop_front())
}

/// Return whether stdin became ready before `timeout_ms`.
///
/// Returns `true` when `poll` woke up before the timeout for any stdin read
/// event, and `false` when the timeout elapsed first or readiness did not
/// include input bytes.
fn poll_readable(stdin: &Stdin, timeout_ms: i32) -> io::Result<bool> {
    let outcome = poll_fd(stdin, timeout_ms)?;
    Ok(outcome.ready && (outcome.revents & libc::POLLIN) != 0)
}

/// Owning handle over the process input stream used by the event loop.
pub(super) struct InputSource {
    stdin: Stdin,
}

impl InputSource {
    /// Open one handle over the process input stream.
    pub(super) fn new() -> Self {
        Self { stdin: stdin() }
    }

    /// Borrow this source as the bounded handle passed to the input decoders.
    pub(super) fn bounded(&self) -> BoundedInput<'_> {
        BoundedInput { stdin: &self.stdin }
    }

    /// Wait without a deadline for the first byte of the next input event.
    ///
    /// This is the event loop's idle wait: no frame is queued and no decode is
    /// in flight, so blocking here costs nothing. It is deliberately absent from
    /// [`BoundedInput`], because blocking part-way through decoding one event
    /// freezes the screen until the terminal finishes a sequence it may never
    /// finish.
    pub(super) fn wait_for_byte(&self) -> io::Result<u8> {
        if let Some(byte) = pop_pending_byte() {
            return Ok(byte);
        }

        unsafe_io::read_byte(&self.stdin)
    }

    /// Attempt one read that reports "no data" instead of waiting.
    ///
    /// Returns `Some(byte)` when a byte was immediately available, and `None`
    /// when the descriptor had nothing to give. Callers use this after `poll`
    /// reported readiness, which can be spurious on macOS PTY slaves.
    pub(super) fn try_read_byte(&self) -> io::Result<Option<u8>> {
        unsafe_io::try_read_byte(&self.stdin)
    }

    /// Return whether stdin became ready before `timeout_ms`.
    ///
    /// Returns `true` when readable input arrived before the timeout, and
    /// `false` when the timeout elapsed first.
    pub(super) fn poll_readable(&self, timeout_ms: i32) -> io::Result<bool> {
        poll_readable(&self.stdin, timeout_ms)
    }
}

/// Bounded view over one [`InputSource`], handed to the input decoders.
///
/// The stdin handle stays private to this module, so a decoder cannot reach the
/// unbounded wait on [`InputSource`]: making a decode step block again would
/// require changing its signature, not just its body.
pub(super) struct BoundedInput<'a> {
    stdin: &'a Stdin,
}

impl BoundedInput<'_> {
    /// Read one byte after waiting up to the requested timeout.
    ///
    /// Returns `Some(byte)` when a byte arrives before the deadline, and `None`
    /// when the full timeout elapses without any data.
    ///
    /// On macOS, PTY slave file descriptors can fire spurious `POLLIN` events
    /// that cause `poll` to return before any data is actually present.  The
    /// function handles this by re-polling with the remaining budget after each
    /// spurious wakeup rather than treating the first empty read as a timeout.
    pub(super) fn read_byte_within(&self, timeout_ms: i32) -> io::Result<Option<u8>> {
        if pending_queue_has_bytes() {
            return Ok(pop_pending_byte());
        }

        // Track the deadline so each spurious wakeup consumes only the time
        // that actually elapsed, keeping the full timeout available for real data.
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);

        loop {
            // Recompute remaining budget on every iteration so spurious wakeups
            // do not eat into the timeout beyond the scheduler jitter they cause.
            let remaining_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(i32::MAX as u128) as i32;

            if !poll_readable(self.stdin, remaining_ms)? {
                // poll timed out with no readiness: the full budget is exhausted.
                return Ok(None);
            }

            // poll reported POLLIN; attempt a non-blocking read.  On macOS PTY
            // slaves, this can still yield nothing (spurious wakeup), in which
            // case the loop retries with the remaining deadline.
            if let Some(byte) = unsafe_io::try_read_byte(self.stdin)? {
                return Ok(Some(byte));
            }

            // Spurious wakeup: check whether the deadline has passed before
            // polling again to avoid an infinite loop on a permanently
            // misbehaving file descriptor.
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }
}

/// Replace the thread-local lookahead queue with `bytes` for one test.
#[cfg(test)]
pub(super) fn queue_pending_bytes(bytes: &[u8]) {
    PENDING_BYTES.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.clear();
        queue.extend(bytes.iter().copied());
    });
}

/// Drop every queued lookahead byte so one test cannot leak state into the next.
#[cfg(test)]
pub(super) fn clear_pending_bytes() {
    PENDING_BYTES.with(|queue| queue.borrow_mut().clear());
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "test fixtures wait for helpers they control, with no event loop to stall"
)]
mod tests {
    use super::*;

    /// Verify that a byte written to the PTY master within the timeout window is
    /// returned even after a spurious `POLLIN` wakeup that yields no data.
    ///
    /// On macOS, polling an empty PTY slave fd fires `POLLIN` immediately even
    /// when no data is present.  `read_byte_within` must retry the poll for the
    /// remaining budget instead of returning `None` on the first spurious
    /// wakeup.  This test fails without the retry loop because a spurious wakeup
    /// causes the function to return `None` and miss the byte that arrives
    /// within the timeout budget.
    ///
    /// The writer delay (20 ms) is deliberately short so the byte arrives well
    /// before the first spurious wakeup has a chance to exhaust the timeout, while
    /// the timeout itself (2 000 ms) is large enough to absorb scheduler latency
    /// on heavily loaded CI machines.
    #[test]
    fn test_read_byte_within_retries_after_spurious_pollin() {
        use super::super::super::unsafe_io::{
            PtyPair, StdinGuard, redirect_stdin_to_fd, set_raw_mode_fd, write_byte_to_fd,
        };
        use std::os::fd::AsRawFd;
        use std::sync::{Mutex, OnceLock};
        use std::thread;

        // Serialize all tests that redirect fd 0 so they cannot interfere with
        // each other when the test harness runs unit tests in parallel.
        static STDIN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = STDIN_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let pty = PtyPair::open().expect("open pty pair");
        // Switch the slave to raw mode so individual bytes written to the master
        // are delivered immediately.  In the default canonical mode the PTY line
        // discipline buffers input until a newline, so the test byte would never
        // arrive at the reader within the timeout window.
        set_raw_mode_fd(&pty.slave).expect("set raw mode on pty slave");
        // The guard restores fd 0 when it drops, whether on normal return or panic.
        let _stdin_guard: StdinGuard =
            redirect_stdin_to_fd(&pty.slave).expect("redirect stdin to pty slave");

        // Write a byte to the master from a separate thread after a short delay.
        // The delay ensures the PTY slave read buffer is empty when
        // `read_byte_within` first polls, triggering the spurious POLLIN path on
        // macOS before the real byte has arrived.
        let writer = thread::spawn({
            // Capture the raw fd integer by value; `pty` outlives the thread
            // join below, so the underlying file descriptor remains open and
            // valid for the duration of the write.
            let master_fd = pty.master.as_raw_fd();
            move || {
                thread::sleep(Duration::from_millis(20));
                write_byte_to_fd(master_fd, b'[').expect("write to pty master");
            }
        });

        let source = InputSource::new();
        // Use a 2 000 ms timeout so the test passes even when the CI scheduler
        // delays the writer thread well beyond its nominal 20 ms sleep.  Without
        // the retry loop a single spurious POLLIN would still return None
        // immediately regardless of how large this timeout is.
        let result = source
            .bounded()
            .read_byte_within(2_000)
            .expect("read_byte_within must not error");

        writer.join().expect("writer thread must not panic");

        assert_eq!(
            result,
            Some(b'['),
            "byte written within the timeout window must be returned, not dropped on spurious POLLIN"
        );
    }

    /// Verify that queued lookahead bytes are consumed before stdin is polled.
    #[test]
    fn test_read_byte_within_drains_pending_queue_first() {
        queue_pending_bytes(b"ab");
        let source = InputSource::new();
        assert_eq!(
            source
                .bounded()
                .read_byte_within(0)
                .expect("read queued byte"),
            Some(b'a')
        );
        clear_pending_bytes();
    }
}
