//! Buffer-word completion candidate extraction.

use super::{CompletionCandidate, CompletionRequest, CompletionSourceId, normalize_text};
use crate::navigation::is_word_char;
use crate::text_buffer::TextBuffer;
use std::collections::HashSet;

/// Collect completion candidates from words already present in the active buffer.
pub(crate) fn collect_buffer_candidates(
    request: &CompletionRequest,
    buffer: &TextBuffer,
) -> Vec<CompletionCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut current = String::new();

    // Scan rope chunks by byte so ASCII-heavy buffers avoid a full `char_at` walk.
    for chunk in buffer.chunks() {
        let bytes = chunk.as_bytes();
        let mut byte_idx = 0usize;
        while byte_idx < bytes.len() {
            let byte = bytes[byte_idx];
            if byte.is_ascii() {
                if ascii_is_word_byte(byte) {
                    current.push(byte as char);
                } else {
                    let rank = candidates.len();
                    push_candidate(&mut candidates, &mut seen, &current, request, rank);
                    current.clear();
                }
                byte_idx += 1;
                continue;
            }

            let Some(ch) = chunk[byte_idx..].chars().next() else {
                break;
            };
            if is_word_char(ch) {
                current.push(ch);
            } else {
                let rank = candidates.len();
                push_candidate(&mut candidates, &mut seen, &current, request, rank);
                current.clear();
            }
            byte_idx += ch.len_utf8();
        }
    }
    let rank = candidates.len();
    push_candidate(&mut candidates, &mut seen, &current, request, rank);

    candidates
}

/// Conditionally add one scanned word as a completion candidate.
fn push_candidate(
    candidates: &mut Vec<CompletionCandidate>,
    seen: &mut HashSet<String>,
    word: &str,
    request: &CompletionRequest,
    rank: usize,
) {
    if word.is_empty() {
        return;
    }

    let normalized_word = normalize_text(word);
    if word.chars().count() < request.min_candidate_length
        || normalized_word.len() <= request.normalized_prefix.len()
        || !normalized_word.starts_with(request.normalized_prefix.as_str())
        || !seen.insert(normalized_word.clone())
    {
        return;
    }

    candidates.push(CompletionCandidate {
        source_id: CompletionSourceId::BufferText,
        insert_text: word.to_string(),
        replace_start_char_idx: request.prefix_start_char_idx,
        replace_end_char_idx: request.cursor_char_idx,
        rank,
    });
}

/// Return whether one ASCII byte belongs to an identifier-like word.
fn ascii_is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::build_request;

    /// Build one request used by the buffer-source unit tests.
    fn request_for(text: &str, cursor_char_idx: usize) -> CompletionRequest {
        let buffer = TextBuffer::from_str(text);
        build_request(&buffer, 1, cursor_char_idx, 1).expect("request should exist")
    }

    #[test]
    /// Confirm case-insensitive prefixes still produce original-case candidates.
    fn test_collect_buffer_candidates_matches_case_insensitively() {
        let text = "Message message mesh";
        let buffer = TextBuffer::from_str(text);
        let request = request_for(text, 2);
        let candidates = collect_buffer_candidates(&request, &buffer);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.insert_text.as_str())
                .collect::<Vec<_>>(),
            vec!["Message", "mesh"]
        );
    }

    #[test]
    /// Confirm duplicate case variants collapse to one visible suggestion.
    fn test_collect_buffer_candidates_collapses_duplicate_case_variants() {
        let text = "Buffer buffer BUFFER buffered";
        let buffer = TextBuffer::from_str(text);
        let request = request_for(text, 2);
        let candidates = collect_buffer_candidates(&request, &buffer);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.insert_text.as_str())
                .collect::<Vec<_>>(),
            vec!["Buffer", "buffered"]
        );
    }

    #[test]
    /// Confirm completions only extend the typed prefix instead of repeating it.
    fn test_collect_buffer_candidates_skips_equal_length_matches() {
        let text = "map Map mapping";
        let buffer = TextBuffer::from_str(text);
        let request = request_for(text, 3);
        let candidates = collect_buffer_candidates(&request, &buffer);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.insert_text.as_str())
                .collect::<Vec<_>>(),
            vec!["mapping"]
        );
    }

    #[test]
    /// Confirm the source scans the whole buffer instead of truncating at a fixed bound.
    fn test_collect_buffer_candidates_scans_without_fixed_limit() {
        let repeated = "alphabet ".repeat(128);
        let buffer = TextBuffer::from_str(&repeated);
        let request = build_request(&buffer, 1, 2, 1).expect("request should exist");
        let candidates = collect_buffer_candidates(&request, &buffer);

        assert_eq!(candidates.len(), 1);
    }
}
