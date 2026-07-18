//! Shared text normalization for whole-string phrase keys.
//!
//! Used by IM bare-text command phrases and whats-next end-option filtering:
//! drop every character that is not alphanumeric (whitespace, ASCII/full-width
//! punctuation, symbols), lowercase letters, then compare as a whole string.

/// Normalize `text` into a phrase/end-option key.
pub fn normalize_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_whitespace_and_fullwidth_punctuation() {
        assert_eq!(normalize_key("新建 会话！"), "新建会话");
        assert_eq!(normalize_key("End this turn."), "endthisturn");
        assert_eq!(normalize_key("  状态。 "), "状态");
        assert_eq!(normalize_key("todo-rm"), "todorm");
        assert_eq!(normalize_key("We're done"), "weredone");
    }
}
