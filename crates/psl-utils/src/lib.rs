use std::borrow::Cow;

/// Normalize a domain for use with [`domain_str`] / [`suffix_str`]:
/// strip a single trailing dot (FQDN form) and lowercase any ASCII upper
/// bytes. Borrows when the input is already normalized; only allocates
/// when uppercase bytes are present.
pub fn normalize_domain(s: &str) -> Cow<'_, str> {
    let trimmed = s.strip_suffix('.').unwrap_or(s);
    if trimmed.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(trimmed.to_ascii_lowercase())
    } else {
        Cow::Borrowed(trimmed)
    }
}

/// Look up the organizational (registrable) domain.
/// The caller is expected to pass an already-normalized domain (typically
/// via [`normalize_domain`]); the returned slice borrows from `s`.
pub fn domain_str(s: &str) -> Option<&str> {
    psl::domain_str(s)
}

/// Look up the public suffix.
/// The caller is expected to pass an already-normalized domain (typically
/// via [`normalize_domain`]); the returned slice borrows from `s`.
pub fn suffix_str(s: &str) -> Option<&str> {
    psl::suffix_str(s)
}

/// Check whether the public suffix is explicitly present in the public
/// suffix list (as opposed to being resolved via the fallback rule for an
/// unknown top-level domain). Returns `false` when the input has no public
/// suffix. The caller is expected to pass an already-normalized domain
/// (typically via [`normalize_domain`]).
pub fn suffix_is_known(s: &str) -> bool {
    psl::suffix(s.as_bytes())
        .map(|suffix| suffix.is_known())
        .unwrap_or(false)
}

/// Check whether the registrable domain's public suffix is explicitly
/// present in the public suffix list. Returns `false` when the input has no
/// registrable domain. The caller is expected to pass an already-normalized
/// domain (typically via [`normalize_domain`]).
pub fn domain_is_known(s: &str) -> bool {
    psl::domain(s.as_bytes())
        .map(|domain| domain.suffix().is_known())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_normalized_borrows() {
        let s = "example.com";
        let n = normalize_domain(s);
        assert!(matches!(n, Cow::Borrowed(_)));
        assert_eq!(n, "example.com");
    }

    #[test]
    fn trims_trailing_dot_without_alloc() {
        let n = normalize_domain("example.com.");
        assert!(matches!(n, Cow::Borrowed(_)));
        assert_eq!(n, "example.com");
    }

    #[test]
    fn lowercases_when_needed() {
        let n = normalize_domain("Example.COM");
        assert!(matches!(n, Cow::Owned(_)));
        assert_eq!(n, "example.com");
    }

    #[test]
    fn trailing_dot_and_uppercase() {
        let n = normalize_domain("Example.COM.");
        assert_eq!(n, "example.com");
    }

    #[test]
    fn empty() {
        assert_eq!(normalize_domain(""), "");
    }

    #[test]
    fn domain_str_after_normalize() {
        let n = normalize_domain("foo.Example.COM.");
        assert_eq!(domain_str(&n), Some("example.com"));
    }

    #[test]
    fn suffix_is_known_known() {
        let n = normalize_domain("www.example.com");
        assert_eq!(suffix_is_known(&n), true);
    }

    #[test]
    fn suffix_is_known_unknown_fallback() {
        let n = normalize_domain("domain.invalid");
        assert_eq!(suffix_is_known(&n), false);
    }

    #[test]
    fn suffix_is_known_empty() {
        let n = normalize_domain("");
        assert_eq!(suffix_is_known(&n), false);
    }

    #[test]
    fn domain_is_known_known() {
        let n = normalize_domain("www.example.com");
        assert_eq!(domain_is_known(&n), true);
    }

    #[test]
    fn domain_is_known_unknown_fallback() {
        let n = normalize_domain("domain.invalid");
        assert_eq!(domain_is_known(&n), false);
    }

    #[test]
    fn domain_is_known_empty() {
        let n = normalize_domain("");
        assert_eq!(domain_is_known(&n), false);
    }
}
