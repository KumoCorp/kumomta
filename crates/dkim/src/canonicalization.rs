use crate::hash::LimitHasher;
use bstr::ByteSlice;
use memchr::memmem::Finder;
use std::sync::LazyLock;

#[derive(PartialEq, Clone, Debug, Copy)]
pub enum Type {
    Simple,
    Relaxed,
}

impl Type {
    pub fn canon_name(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Relaxed => "relaxed",
        }
    }

    pub(crate) fn canon_body(&self, body: &[u8], hasher: &mut LimitHasher) {
        match self {
            Self::Simple => body_simple(body, hasher),
            Self::Relaxed => body_relaxed(body, hasher),
        }
    }

    pub(crate) fn canon_header_into(&self, key: &[u8], value: &[u8], out: &mut Vec<u8>) {
        match self {
            Self::Simple => canonicalize_header_simple(key, value, out),
            Self::Relaxed => canonicalize_header_relaxed(key, value, out),
        }
    }
}

fn do_body_simple(mut body: &[u8]) -> &[u8] {
    if body.is_empty() {
        return b"\r\n";
    }

    while body.ends_with(b"\r\n\r\n") {
        body = &body[..body.len() - 2];
    }

    body
}

/// Canonicalize body using the simple canonicalization algorithm.
fn body_simple(body: &[u8], hasher: &mut LimitHasher) {
    let body = do_body_simple(body);
    hasher.hash(body);
}

/// Helper for iterating lines using memmem
struct IterLines<'haystack> {
    haystack: &'haystack [u8],
    inner: memchr::memmem::FindIter<'haystack, 'static>,
    start: usize,
    done: bool,
}

impl<'haystack> Iterator for IterLines<'haystack> {
    type Item = &'haystack [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        match self.inner.next() {
            Some(idx) => {
                let line = &self.haystack[self.start..idx + 2];
                self.start = idx + 2;
                Some(line)
            }
            None => {
                self.done = true;
                let line = &self.haystack[self.start..];
                if line.is_empty() {
                    None
                } else {
                    Some(line)
                }
            }
        }
    }
}

fn iter_lines(haystack: &'_ [u8]) -> IterLines<'_> {
    static CRLF: LazyLock<Finder> = LazyLock::new(|| memchr::memmem::Finder::new("\r\n"));
    IterLines {
        haystack,
        inner: CRLF.find_iter(haystack),
        start: 0,
        done: false,
    }
}

/// Canonicalize a body using the relaxed canonicalization algorithm from
/// RFC 6376 section 3.4.4. That section refers to section 3.4.3 only for
/// the definition of an "empty line"; the relaxed algorithm itself is
/// specified in section 3.4.4.
/// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.4
fn body_relaxed(body: &[u8], hasher: &mut LimitHasher) {
    let mut pending_empty_lines = 0usize;

    for mut line in iter_lines(body) {
        // Ignore all whitespace at the end of the line
        line = trim_ws_end(line);

        // Empty lines are only emitted if followed by a non-empty line. This
        // drops all trailing empty lines, including a body consisting solely
        // of CRLF, as required by RFC 6376 section 3.4.4.
        if line.is_empty() {
            pending_empty_lines += 1;
            continue;
        }

        for _ in 0..pending_empty_lines {
            hasher.hash(b"\r\n");
        }
        pending_empty_lines = 0;

        let mut prior = 0;
        // Reduce all sequences of WSP within a line to a single SP character.
        for idx in memchr::memchr2_iter(b' ', b'\t', line) {
            if prior > 0 && idx == prior {
                // Part of a run; ignore this one
                prior = idx + 1;
                continue;
            }

            // Found a new run of space(s).
            // Emit the bytes ahead of this one
            hasher.hash(&line[prior..idx]);
            // and emit the canonical space
            hasher.hash(b" ");

            prior = idx + 1;
        }
        // and emit the remainder
        hasher.hash(&line[prior..]);

        // and canonical newline
        hasher.hash(b"\r\n");
    }
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.1
fn canonicalize_header_simple(key: &[u8], value: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(key);
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.extend_from_slice(b"\r\n");
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.2
fn canonicalize_header_relaxed(key: &[u8], value: &[u8], out: &mut Vec<u8>) {
    let key = key.to_ascii_lowercase();
    let key = key.trim_end();

    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(b":");

    let value = trim_ws_start(trim_ws_end(value));
    let mut space_run = false;
    for &c in value {
        match c {
            b'\r' | b'\n' => {}
            b' ' | b'\t' => {
                if space_run {
                    continue;
                }
                space_run = true;
                out.push(b' ');
            }
            _ => {
                space_run = false;
                out.push(c);
            }
        }
    }

    out.extend_from_slice(b"\r\n");
}

fn trim_ws_start(mut line: &[u8]) -> &[u8] {
    while let Some(c) = line.first() {
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => line = &line[1..],
            _ => break,
        }
    }
    line
}

fn trim_ws_end(mut line: &[u8]) -> &[u8] {
    while let Some(c) = line.last() {
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                line = &line[0..line.len() - 1];
            }
            _ => break,
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_relaxed(key: &str, value: &[u8]) -> Vec<u8> {
        let mut result = vec![];
        canonicalize_header_relaxed(key.as_bytes(), value, &mut result);
        result
    }

    #[test]
    fn test_canonicalize_header_relaxed() {
        assert_eq!(header_relaxed("SUBJect", b" AbC\r\n"), b"subject:AbC\r\n");
        assert_eq!(
            header_relaxed("Subject \t", b"\t Your Name\t \r\n"),
            b"subject:Your Name\r\n"
        );
        assert_eq!(
            header_relaxed("Subject \t", b"\t Kimi \t \r\n No \t\r\n Na Wa\r\n"),
            b"subject:Kimi No Na Wa\r\n"
        );
    }

    fn body_relaxed(data: &[u8]) -> Vec<u8> {
        body_relaxed_with_limit(data, usize::MAX)
    }

    fn body_relaxed_with_limit(data: &[u8], limit: usize) -> Vec<u8> {
        let mut hasher = LimitHasher {
            hasher: crate::hash::HashImpl::copy_data(),
            limit,
            hashed: 0,
        };
        super::body_relaxed(data, &mut hasher);
        hasher.finalize_bytes()
    }

    /// A deliberately non-streaming implementation of RFC 6376 section
    /// 3.4.4 for checking the production streaming implementation.
    fn reference_body_relaxed(body: &[u8]) -> Vec<u8> {
        let mut lines = vec![];
        let mut start = 0;

        while let Some(offset) = body[start..]
            .windows(2)
            .position(|window| window == b"\r\n")
        {
            let end = start + offset;
            lines.push(&body[start..end]);
            start = end + 2;
        }

        if start < body.len() {
            lines.push(&body[start..]);
        }

        let mut canonical_lines = lines
            .into_iter()
            .map(|line| {
                let mut canonical = vec![];
                let mut pending_wsp = false;

                for &byte in line {
                    match byte {
                        b' ' | b'\t' => pending_wsp = true,
                        _ => {
                            if pending_wsp {
                                canonical.push(b' ');
                                pending_wsp = false;
                            }
                            canonical.push(byte);
                        }
                    }
                }

                canonical
            })
            .collect::<Vec<_>>();

        while canonical_lines.last().is_some_and(Vec::is_empty) {
            canonical_lines.pop();
        }

        let mut canonical = canonical_lines
            .into_iter()
            .collect::<Vec<_>>()
            .join(b"\r\n".as_slice());
        if !canonical.is_empty() {
            canonical.extend_from_slice(b"\r\n");
        }
        canonical
    }

    fn body_simple(data: &[u8]) -> Vec<u8> {
        let mut hasher = LimitHasher {
            hasher: crate::hash::HashImpl::copy_data(),
            limit: usize::MAX,
            hashed: 0,
        };
        super::body_simple(data, &mut hasher);
        hasher.finalize_bytes()
    }

    #[test]
    fn test_canonicalize_body_relaxed() {
        assert_eq!(body_relaxed(b""), b"");
        assert_eq!(body_relaxed(b"\r\n"), b"");
        assert_eq!(body_relaxed(b"\r\n\r\n"), b"");
        assert_eq!(body_relaxed(b" \t\r\n\t\r\n"), b"");
        assert_eq!(body_relaxed(b"hey        \r\n"), b"hey\r\n");
        assert_eq!(body_relaxed(b" C \r\nD \t E\r\n\r\n\r\n"), b" C\r\nD E\r\n");
        assert_eq!(body_relaxed(b"\r\nhey \t\r\n \t\r\n"), b"\r\nhey\r\n");
        assert_eq!(
            body_relaxed(b"hey\r\n \t\r\nthere"),
            b"hey\r\n\r\nthere\r\n"
        );
    }

    #[test]
    fn test_canonicalize_body_relaxed_against_reference() {
        let line_variants: &[&[u8]] = &[
            b"",
            b"a",
            b" ",
            b"\t",
            b" \t",
            b"a ",
            b" a",
            b"a \t b",
            b" a\t \tb ",
        ];

        for line_count in 0..=4u32 {
            let combination_count = line_variants.len().pow(line_count);

            for combination in 0..combination_count {
                for terminated in [false, true] {
                    let mut selected = combination;
                    let mut body = vec![];

                    for line_index in 0..line_count {
                        if line_index > 0 {
                            body.extend_from_slice(b"\r\n");
                        }
                        body.extend_from_slice(line_variants[selected % line_variants.len()]);
                        selected /= line_variants.len();
                    }

                    if terminated {
                        body.extend_from_slice(b"\r\n");
                    }

                    assert_eq!(
                        body_relaxed(&body),
                        reference_body_relaxed(&body),
                        "body={body:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_canonicalize_body_relaxed_respects_length_limit() {
        let body = b"\r\n C \t\r\n \t\r\nD \t E\r\n\r\n";
        let canonical = reference_body_relaxed(body);

        for limit in 0..=canonical.len() + 1 {
            assert_eq!(
                body_relaxed_with_limit(body, limit),
                &canonical[..limit.min(canonical.len())],
                "limit={limit}"
            );
        }
    }

    #[test]
    fn test_canonicalize_body_simple() {
        assert_eq!(body_simple(b"\r\n"), b"\r\n");
        assert_eq!(body_simple(b"hey        \r\n"), b"hey        \r\n");
        assert_eq!(
            body_simple(b" C \r\nD \t E\r\n\r\n\r\n"),
            b" C \r\nD \t E\r\n"
        );
    }
}
