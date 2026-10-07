//! A small, deterministic subsequence matcher. No filesystem work, regex, or index.

/// Score an already-lowercased name against an already-lowercased query.
/// Prefixes, adjacent characters, and word boundaries win; lower scores win.
pub fn score(name: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let mut wanted = query.chars().peekable();
    let mut cost = 0usize;
    let mut previous_match = None;
    let mut boundary = true;
    for (position, ch) in name.chars().enumerate() {
        if wanted.peek().copied() == Some(ch) {
            wanted.next();
            match previous_match {
                None => cost += position * 4,
                Some(previous) => cost += (position - previous - 1) * 3,
            }
            if !boundary && previous_match != position.checked_sub(1) {
                cost += 2;
            }
            previous_match = Some(position);
            if wanted.peek().is_none() {
                return Some(cost);
            }
        }
        boundary = matches!(ch, '-' | '_' | ' ' | '.' | '/' | '\\');
    }
    None
}

/// Neutralize terminal escapes, line breaks, and directional formatting controls.
/// Never use this string as a filesystem path or shell command.
pub fn safe_label(input: &str) -> String {
    let mut label = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control()
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            label.extend(ch.escape_debug());
        } else {
            label.push(ch);
        }
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_prefix_then_compact_matches() {
        assert!(score("src", "sr") < score("source", "sr"));
        assert!(score("src", "sr") < score("my-src", "sr"));
        assert!(score("abcdef", "acf").is_some());
        assert_eq!(score("abcdef", "fca"), None);
        assert_eq!(score("anything", ""), Some(0));
    }

    #[test]
    fn unicode_subsequences_are_not_byte_matches() {
        assert!(score("räksmörgås", "räå").is_some());
        assert!(score("日本語", "日語").is_some());
        assert_eq!(score("ö", "o"), None);
    }

    #[test]
    fn hostile_labels_cannot_control_the_terminal() {
        let escaped = safe_label("hello\x1b[2J\nworld\t\u{202e}txt");
        assert!(!escaped.chars().any(char::is_control));
        assert!(!escaped.contains('\u{202e}'));
        assert!(escaped.contains("\\n"));
        assert_eq!(safe_label("räv / 日本 / 🦊"), "räv / 日本 / 🦊");
    }
}
