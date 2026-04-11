//! LSP lookup token allocation and in-flight request metadata.

use crate::lsp::NavigationKind;

/// Shared monotonic token source for editor-owned LSP lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LookupTokenSource {
    /// Next token returned to a queued lookup request.
    next_token: u64,
}

impl LookupTokenSource {
    /// Create one token source with the first usable lookup token.
    pub(super) fn new() -> Self {
        Self { next_token: 1 }
    }

    /// Allocate the next monotonic lookup token.
    pub(super) fn next(&mut self) -> u64 {
        let token = self.next_token;
        // Lookup tokens are matched only against currently stored request state,
        // so wrapping back to the first usable token avoids getting stuck.
        self.next_token = if token == u64::MAX { 1 } else { token + 1 };
        token
    }
}

impl Default for LookupTokenSource {
    /// Create the default shared lookup token source.
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata for one in-flight navigation request tied to one buffer snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActiveNavigationLookup {
    /// Navigation command that produced this request.
    pub(super) kind: NavigationKind,
    /// Monotonic token used to reject stale navigation responses.
    pub(super) token: u64,
    /// Buffer version captured when the navigation request was queued.
    pub(super) document_version: i32,
}

/// Metadata for one in-flight hover request tied to the active buffer snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActiveHoverLookup {
    /// Monotonic token used to reject stale hover responses from older requests.
    pub(super) token: u64,
    /// Buffer version captured when the hover request was queued.
    pub(super) document_version: i32,
}

/// Metadata for one in-flight rename request tied to the active buffer snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveRenameLookup {
    /// Monotonic token used to reject stale rename responses from older requests.
    pub(super) token: u64,
    /// Buffer version captured when the rename request was queued.
    pub(super) document_version: i32,
    /// Global edit generation captured when the rename request was queued.
    pub(super) request_edit_generation: u64,
    /// Replacement symbol name associated with the request.
    pub(super) new_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Lookup token allocation should wrap to the first usable value after `u64::MAX`.
    fn test_lookup_token_source_wraps_after_max() {
        let mut source = LookupTokenSource {
            next_token: u64::MAX,
        };

        assert_eq!(source.next(), u64::MAX);
        assert_eq!(source.next(), 1);
    }
}
