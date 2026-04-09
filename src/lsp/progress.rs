//! Tracking and formatting for in-flight LSP progress notifications.

use super::protocol::LspProgressNotification;
use crate::spinner::Spinner;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Maximum number of progress body lines shown above the footer line.
const MAX_VISIBLE_PROGRESS_ENTRIES: usize = 3;
/// Number of quiet polls an active progress entry may survive without another
/// notification before Ordex treats it as stale and clears the overlay line.
const ACTIVE_PROGRESS_STALE_POLLS: u8 = 18;
/// Number of idle polls that keep a completed entry visible so a quick begin/end
/// sequence can still paint an overlay before the line disappears.
const COMPLETED_PROGRESS_GRACE_POLLS: u8 = 4;
/// Label shown in the footer line so the overlay identifies the active LSP.
const LSP_OVERLAY_NAME: &str = "rust-analyzer";

/// One progress notification paired with its workspace root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LspProgressEvent {
    /// Workspace root that owns the token namespace for this notification.
    pub(crate) workspace_root: PathBuf,
    /// Typed progress payload decoded from one `$/progress` message.
    pub(crate) notification: LspProgressNotification,
}

/// One bounded tracker that converts LSP progress notifications into UI lines.
#[derive(Debug, Default)]
pub(crate) struct ProgressTracker {
    workspaces: HashMap<PathBuf, WorkspaceProgress>,
    recent_entries: Vec<RecentProgressEntry>,
    /// Monotonic sequence assigned to each incoming progress update so the tracker
    /// can keep newest-visible entries ordered without consulting wall-clock time.
    next_update_order: u64,
    /// Spinner advanced while the overlay stays visible.
    spinner: Spinner,
}

#[derive(Debug, Default)]
struct WorkspaceProgress {
    entries: HashMap<String, ProgressEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgressEntry {
    title: String,
    message: Option<String>,
    percentage: Option<u8>,
    /// Monotonic sequence captured when this entry was last updated so visible
    /// lines can be sorted by freshness while keeping deterministic ordering.
    update_order: u64,
    /// Remaining quiet polls before this active entry is treated as stale.
    remaining_idle_polls: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One completed progress line retained for a short grace window after its end
/// event arrives so fast completions remain visible in the terminal.
struct RecentProgressEntry {
    workspace_root: PathBuf,
    token: String,
    entry: ProgressEntry,
    remaining_polls: u8,
}

impl ProgressTracker {
    /// Apply one progress event and return the current visible overlay lines.
    pub(crate) fn apply(&mut self, event: LspProgressEvent) -> Vec<String> {
        let update_order = self.allocate_update_order();
        self.advance_spinner_frame();
        match event.notification {
            LspProgressNotification::Begin {
                token,
                title,
                message,
                percentage,
            } => {
                self.discard_recent_entry(&event.workspace_root, &token);
                let workspace = self
                    .workspaces
                    .entry(event.workspace_root.clone())
                    .or_default();
                workspace.entries.insert(
                    token,
                    ProgressEntry {
                        title,
                        message,
                        percentage,
                        update_order,
                        remaining_idle_polls: ACTIVE_PROGRESS_STALE_POLLS,
                    },
                );
            }
            LspProgressNotification::Report {
                token,
                message,
                percentage,
            } => {
                self.discard_recent_entry(&event.workspace_root, &token);
                let workspace = self
                    .workspaces
                    .entry(event.workspace_root.clone())
                    .or_default();
                let entry = workspace
                    .entries
                    .entry(token)
                    .or_insert_with(ProgressEntry::placeholder);
                // Reports mutate only the fields the server sent, so an omitted
                // message does not erase the already visible task label.
                if let Some(message) = message {
                    entry.message = Some(message);
                }
                if let Some(percentage) = percentage {
                    entry.percentage = Some(percentage);
                }
                entry.update_order = update_order;
                entry.remaining_idle_polls = ACTIVE_PROGRESS_STALE_POLLS;
            }
            LspProgressNotification::End { token, .. } => {
                let workspace = self
                    .workspaces
                    .entry(event.workspace_root.clone())
                    .or_default();
                if let Some(mut entry) = workspace.entries.remove(&token) {
                    entry.update_order = update_order;
                    self.recent_entries.push(RecentProgressEntry {
                        workspace_root: event.workspace_root.clone(),
                        token,
                        entry,
                        remaining_polls: COMPLETED_PROGRESS_GRACE_POLLS,
                    });
                }
            }
        }
        if self
            .workspaces
            .get(&event.workspace_root)
            .is_some_and(|workspace| workspace.entries.is_empty())
        {
            self.workspaces.remove(&event.workspace_root);
        }
        self.overlay_lines()
    }

    /// Advance one quiet overlay poll and return the visible lines.
    pub(crate) fn poll_visible_lines(&mut self) -> Vec<String> {
        if self.has_active_entries() {
            self.advance_spinner_frame();
        }
        // Quiet polls age both recent completions and active entries that have
        // gone silent so the overlay reflects fresh progress instead of stale text.
        for workspace in self.workspaces.values_mut() {
            for entry in workspace.entries.values_mut() {
                entry.remaining_idle_polls = entry.remaining_idle_polls.saturating_sub(1);
            }
            workspace
                .entries
                .retain(|_, entry| entry.remaining_idle_polls > 0);
        }
        self.workspaces
            .retain(|_, workspace| !workspace.entries.is_empty());
        for entry in &mut self.recent_entries {
            entry.remaining_polls = entry.remaining_polls.saturating_sub(1);
        }
        self.recent_entries
            .retain(|entry| entry.remaining_polls > 0);
        self.overlay_lines()
    }

    /// Return whether the tracker still has any lines that should keep polling alive.
    pub(crate) fn has_visible_lines(&self) -> bool {
        self.has_active_entries() || !self.recent_entries.is_empty()
    }

    /// Return the currently visible overlay lines in stable render order.
    pub(crate) fn overlay_lines(&self) -> Vec<String> {
        let include_workspace = self.visible_workspace_count() > 1;
        let mut entries = Vec::new();
        for (root, workspace) in &self.workspaces {
            for entry in workspace.entries.values() {
                entries.push((
                    entry.update_order,
                    format_progress_line(root, entry, include_workspace),
                ));
            }
        }
        for recent in &self.recent_entries {
            entries.push((
                recent.entry.update_order,
                format_progress_line(&recent.workspace_root, &recent.entry, include_workspace),
            ));
        }
        // The overlay shows the freshest tasks while preserving top-to-bottom
        // reading order inside the overlay.
        entries.sort_by_key(|(order, _)| Reverse(*order));
        entries.truncate(MAX_VISIBLE_PROGRESS_ENTRIES);
        entries.sort_by_key(|(order, _)| *order);
        let mut lines = entries
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        if lines.is_empty() {
            return lines;
        }
        lines.push(self.footer_line());
        lines
    }

    /// Return the next monotonic update sequence number.
    fn allocate_update_order(&mut self) -> u64 {
        let order = self.next_update_order;
        self.next_update_order = self.next_update_order.saturating_add(1);
        order
    }

    /// Advance the footer spinner by one frame.
    fn advance_spinner_frame(&mut self) {
        self.spinner.next_frame();
    }

    /// Drop one stale recently completed entry when the same token becomes active again.
    fn discard_recent_entry(&mut self, workspace_root: &Path, token: &str) {
        self.recent_entries
            .retain(|entry| entry.workspace_root != workspace_root || entry.token != token);
    }

    /// Return the footer line that names the LSP and shows its spinner glyph.
    fn footer_line(&self) -> String {
        format!("{LSP_OVERLAY_NAME} {}", self.spinner.current_frame())
    }

    /// Return whether any active progress entry is still tracked.
    fn has_active_entries(&self) -> bool {
        self.workspaces
            .values()
            .any(|workspace| !workspace.entries.is_empty())
    }

    /// Count distinct workspaces across both active and recently completed entries.
    fn visible_workspace_count(&self) -> usize {
        let mut roots = Vec::<&Path>::new();
        for root in self.workspaces.keys().map(PathBuf::as_path) {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        for root in self
            .recent_entries
            .iter()
            .map(|entry| entry.workspace_root.as_path())
        {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots.len()
    }
}

impl ProgressEntry {
    /// Build one fallback entry for out-of-order progress reports.
    fn placeholder() -> Self {
        Self {
            title: "LSP progress".to_string(),
            message: None,
            percentage: None,
            update_order: 0,
            remaining_idle_polls: ACTIVE_PROGRESS_STALE_POLLS,
        }
    }
}

/// Format one progress entry into a user-facing overlay line.
fn format_progress_line(root: &Path, entry: &ProgressEntry, include_workspace: bool) -> String {
    let body = if let Some(message) = entry.message.as_deref() {
        if message == entry.title {
            entry.title.clone()
        } else {
            format!("{}: {message}", entry.title)
        }
    } else {
        entry.title.clone()
    };
    let with_percentage = if let Some(percentage) = entry.percentage {
        format!("{body} ({percentage}%)")
    } else {
        body
    };
    if include_workspace {
        let workspace = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace");
        format!("{workspace}: {with_percentage}")
    } else {
        with_percentage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Return one deterministic workspace root for tracker tests.
    fn workspace(name: &str) -> PathBuf {
        PathBuf::from("/tmp").join(name)
    }

    #[test]
    fn test_progress_tracker_updates_and_clears_one_token() {
        let mut tracker = ProgressTracker::default();

        let lines = tracker.apply(LspProgressEvent {
            workspace_root: workspace("one"),
            notification: LspProgressNotification::Begin {
                token: "cargo-index".to_string(),
                title: "Indexing".to_string(),
                message: Some("crate graph".to_string()),
                percentage: Some(5),
            },
        });
        assert_eq!(lines[0], "Indexing: crate graph (5%)");
        assert!(lines[1].contains("rust-analyzer"));

        let lines = tracker.apply(LspProgressEvent {
            workspace_root: workspace("one"),
            notification: LspProgressNotification::Report {
                token: "cargo-index".to_string(),
                message: Some("macros".to_string()),
                percentage: Some(73),
            },
        });
        assert_eq!(lines[0], "Indexing: macros (73%)");
        assert!(lines[1].contains("rust-analyzer"));

        let lines = tracker.apply(LspProgressEvent {
            workspace_root: workspace("one"),
            notification: LspProgressNotification::End {
                token: "cargo-index".to_string(),
                message: Some("done".to_string()),
            },
        });
        assert_eq!(lines[0], "Indexing: macros (73%)");
        for _ in 0..COMPLETED_PROGRESS_GRACE_POLLS.saturating_sub(1) {
            assert_eq!(tracker.poll_visible_lines()[0], "Indexing: macros (73%)");
        }
        assert!(tracker.poll_visible_lines().is_empty());
    }

    #[test]
    fn test_progress_tracker_limits_visible_lines_to_newest_entries() {
        let mut tracker = ProgressTracker::default();
        for index in 0..4 {
            tracker.apply(LspProgressEvent {
                workspace_root: workspace("one"),
                notification: LspProgressNotification::Begin {
                    token: format!("task-{index}"),
                    title: format!("Task {index}"),
                    message: None,
                    percentage: Some((index * 10) as u8),
                },
            });
        }

        let lines = tracker.overlay_lines();
        assert_eq!(
            &lines[..3],
            &[
                "Task 1 (10%)".to_string(),
                "Task 2 (20%)".to_string(),
                "Task 3 (30%)".to_string(),
            ]
        );
        assert!(lines[3].contains("rust-analyzer"));
    }

    #[test]
    fn test_progress_tracker_prefixes_workspace_when_multiple_roots_are_active() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(LspProgressEvent {
            workspace_root: workspace("alpha"),
            notification: LspProgressNotification::Begin {
                token: "a".to_string(),
                title: "Indexing".to_string(),
                message: None,
                percentage: None,
            },
        });
        let lines = tracker.apply(LspProgressEvent {
            workspace_root: workspace("beta"),
            notification: LspProgressNotification::Begin {
                token: "b".to_string(),
                title: "Diagnostics".to_string(),
                message: Some("workspace load".to_string()),
                percentage: None,
            },
        });

        assert_eq!(
            &lines[..2],
            &[
                "alpha: Indexing".to_string(),
                "beta: Diagnostics: workspace load".to_string(),
            ]
        );
        assert!(lines[2].contains("rust-analyzer"));
    }

    #[test]
    fn test_progress_tracker_keeps_completed_entries_visible_for_grace_polls() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(LspProgressEvent {
            workspace_root: workspace("one"),
            notification: LspProgressNotification::Begin {
                token: "cargo-index".to_string(),
                title: "Indexing".to_string(),
                message: Some("crate graph".to_string()),
                percentage: Some(5),
            },
        });

        let lines = tracker.apply(LspProgressEvent {
            workspace_root: workspace("one"),
            notification: LspProgressNotification::End {
                token: "cargo-index".to_string(),
                message: Some("done".to_string()),
            },
        });
        assert_eq!(lines[0], "Indexing: crate graph (5%)");
        assert!(lines[1].contains("rust-analyzer"));

        for _ in 0..COMPLETED_PROGRESS_GRACE_POLLS.saturating_sub(1) {
            let lines = tracker.poll_visible_lines();
            assert_eq!(lines[0], "Indexing: crate graph (5%)");
        }
        assert!(tracker.poll_visible_lines().is_empty());
    }

    #[test]
    fn test_progress_tracker_drops_silent_active_entries() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(LspProgressEvent {
            workspace_root: workspace("one"),
            notification: LspProgressNotification::Begin {
                token: "cargo-index".to_string(),
                title: "Indexing".to_string(),
                message: Some("crate graph".to_string()),
                percentage: Some(5),
            },
        });

        for _ in 0..ACTIVE_PROGRESS_STALE_POLLS.saturating_sub(1) {
            assert!(!tracker.poll_visible_lines().is_empty());
        }

        assert!(tracker.poll_visible_lines().is_empty());
    }
}
