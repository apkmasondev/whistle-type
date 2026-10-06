//! Safe, meaning-preserving text post-processing.
//!
//! WhistleType never rewrites what the model said. The only transformations are:
//! * control characters (newlines, tabs, ESC, ...) are turned into spaces - always, also in raw mode,
//!   because a newline or an escape sequence pasted into a terminal could execute a command;
//! * in normal mode: trim + collapse runs of whitespace into one space.

/// Replaces every control character with a space. Printable text is untouched.
pub fn neutralize_controls(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// Trim and collapse any run of Unicode whitespace into a single ASCII space.
pub fn normalize_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// True when the text contains at least one letter or digit (punctuation-only output is not speech).
pub fn has_content(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric())
}

/// Joins the transcripts of consecutive audio segments.
pub fn join_segments<S: AsRef<str>>(parts: &[S]) -> String {
    let mut out = String::new();
    for p in parts {
        let p = p.as_ref();
        if p.trim().is_empty() {
            continue;
        }
        if !out.is_empty() && !out.ends_with(char::is_whitespace) && !p.starts_with(char::is_whitespace) {
            out.push(' ');
        }
        out.push_str(p);
    }
    out
}

/// Final text that will be inserted into the target application.
pub fn finalize(model_text: &str, raw: bool, append_space: bool) -> String {
    let safe = neutralize_controls(model_text);
    let mut text = if raw { safe } else { normalize_whitespace(&safe) };
    if append_space && !text.is_empty() && !text.ends_with(' ') {
        text.push(' ');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_whitespace_only() {
        assert_eq!(normalize_whitespace("  Sprawdź   komponent\u{00A0}React \t i  useEffect.  "),
                   "Sprawdź komponent React i useEffect.");
        assert_eq!(normalize_whitespace(""), "");
        assert_eq!(normalize_whitespace("   "), "");
    }

    #[test]
    fn keeps_polish_characters_and_case() {
        let s = "Zażółć gęślą jaźń. ŹDŹBŁO";
        assert_eq!(finalize(s, false, false), s);
        assert_eq!(finalize(s, true, false), s);
    }

    #[test]
    fn raw_mode_keeps_spacing_but_neutralizes_controls() {
        assert_eq!(finalize("  a  b ", true, false), "  a  b ");
        assert_eq!(finalize("rm -rf /\n", true, false), "rm -rf / ");
        assert_eq!(finalize("a\u{1b}[31mb", true, false), "a [31mb");
        assert_eq!(finalize("line1\r\nline2", false, false), "line1 line2");
    }

    #[test]
    fn append_space() {
        assert_eq!(finalize("Test.", false, true), "Test. ");
        assert_eq!(finalize("", false, true), "");
    }

    #[test]
    fn content_detection() {
        assert!(has_content("ok"));
        assert!(has_content("3"));
        assert!(has_content("ąę"));
        assert!(!has_content(" . , ! "));
        assert!(!has_content(""));
    }

    #[test]
    fn joins_segments() {
        assert_eq!(join_segments(&["Pierwsze zdanie.", "", "Drugie zdanie."]), "Pierwsze zdanie. Drugie zdanie.");
        assert_eq!(join_segments::<&str>(&[]), "");
    }
}
