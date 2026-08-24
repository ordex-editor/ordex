//! Const-context string slicing.
//!
//! `str` indexing is not usable in a `const fn` on stable Rust, so the slicing
//! that profile tables need at compile time is done here through raw pointers,
//! behind bounds and char-boundary checks that panic during const evaluation.
#![allow(unsafe_code)]

/// Return a validated UTF-8 slice for one byte range.
///
/// Panics when the range leaves the string or splits a multi-byte character. In
/// a const context that panic is a compile error, so a bad profile table never
/// reaches a running editor.
pub(crate) const fn const_str_range(input: &'static str, start: usize, len: usize) -> &'static str {
    // TODO: Replace this helper with direct string slicing when const `str` indexing is stable on Rust stable.
    let input_len = input.len();
    if start > input_len {
        panic!("const_str_range start exceeds input length");
    }
    if len > input_len - start {
        panic!("const_str_range range exceeds input length");
    }
    let end = start + len;
    if !input.is_char_boundary(start) {
        panic!("const_str_range start must be on a char boundary");
    }
    if !input.is_char_boundary(end) {
        panic!("const_str_range end must be on a char boundary");
    }
    let ptr = input.as_ptr();
    // SAFETY: The range has been checked to stay in-bounds.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.add(start), len) };
    // SAFETY: The source string is valid UTF-8, and the checked range preserves char boundaries.
    unsafe { std::str::from_utf8_unchecked(bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify an in-bounds range returns exactly the requested bytes.
    #[test]
    fn returns_the_requested_range() {
        assert_eq!(const_str_range("*/", 1, 1), "/");
        assert_eq!(const_str_range("<!--", 0, 4), "<!--");
        assert_eq!(const_str_range("abc", 3, 0), "");
    }

    /// Verify multi-byte characters survive when the range respects boundaries.
    #[test]
    fn keeps_multi_byte_characters_intact() {
        assert_eq!(const_str_range("«»", 0, 2), "«");
    }

    /// Verify const evaluation accepts the helper, which is its only real caller.
    #[test]
    fn works_in_a_const_context() {
        const SLICE: &str = const_str_range("/**/", 2, 2);
        assert_eq!(SLICE, "*/");
    }

    /// Verify a range reaching past the end is rejected.
    #[test]
    #[should_panic(expected = "range exceeds input length")]
    fn rejects_a_range_past_the_end() {
        let _ = const_str_range("ab", 1, 5);
    }

    /// Verify a range splitting a multi-byte character is rejected.
    #[test]
    #[should_panic(expected = "char boundary")]
    fn rejects_a_split_character() {
        let _ = const_str_range("«»", 0, 1);
    }
}
