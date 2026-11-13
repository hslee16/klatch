/// Names are restricted so they cannot carry newlines or fake headers into a model's context.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Collapse line breaks so one message stays on one line.
pub fn flat(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// Strip trailing slashes; the empty path becomes "/".
pub fn norm(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.to_string() }
}

/// a overlaps b if either is a path-component prefix of the other.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    let pre = |x: &str, y: &str| x == y || y.starts_with(&format!("{}/", x.trim_end_matches('/')));
    pre(&a, &b) || pre(&b, &a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_accept_the_safe_alphabet_only() {
        assert!(valid_name("alice-1_x.y"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name("a\nb"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(&"x".repeat(41)));
        assert!(valid_name(&"x".repeat(40)));
    }

    #[test]
    fn flat_removes_line_breaks() {
        assert_eq!(flat("a\r\nb\nc"), "a  b c");
    }

    #[test]
    fn norm_strips_trailing_slashes() {
        assert_eq!(norm("src/"), "src");
        assert_eq!(norm("src//"), "src");
        assert_eq!(norm("/"), "/");
        assert_eq!(norm(""), "/");
    }

    #[test]
    fn overlap_is_by_path_component() {
        assert!(overlaps("src", "src/a.rs"));
        assert!(overlaps("src/a.rs", "src"));
        assert!(overlaps("src/", "src"));
        assert!(overlaps("/", "/abs/here"));
        assert!(!overlaps("/", "relative/here")); // root covers absolute paths only
        assert!(!overlaps("src", "srcs/a.rs"));
        assert!(!overlaps("src/a", "src/b"));
    }
}
