use crate::hash::LimitHasher;
use memchr::memmem::Finder;
use once_cell::sync::Lazy;

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

    pub(crate) fn canon_header_into(&self, key: &str, value: &[u8], out: &mut Vec<u8>) {
        match self {
            Self::Simple => canonicalize_header_simple(key, value, out),
            Self::Relaxed => canonicalize_header_relaxed(key, value, out),
        }
    }
}

fn do_body_simple<'a>(mut body: &'a [u8]) -> &'a [u8] {
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

fn iter_lines(haystack: &[u8]) -> IterLines {
    static CRLF: Lazy<Finder> = Lazy::new(|| memchr::memmem::Finder::new("\r\n"));
    IterLines {
        haystack,
        inner: CRLF.find_iter(haystack),
        start: 0,
        done: false,
    }
}

/// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.3
/// Canonicalize body using the relaxed canonicalization algorithm.
fn body_relaxed(mut body: &[u8], hasher: &mut LimitHasher) {
    if body.is_empty() {
        return;
    }

    // Ignore empty lines at the end of the message body
    while body.ends_with(b"\r\n\r\n") {
        body = &body[..body.len() - 2];
    }

    for mut line in iter_lines(body) {
        // Ignore all whitespace at the end of the line
        line = trim_ws_end(line);

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
fn canonicalize_header_simple(key: &str, value: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.extend_from_slice(b"\r\n");
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.2
fn canonicalize_header_relaxed(key: &str, value: &[u8], out: &mut Vec<u8>) {
    let key = key.to_lowercase();
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
        canonicalize_header_relaxed(key, value, &mut result);
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
        let mut hasher = LimitHasher {
            hasher: crate::hash::HashImpl::copy_data(),
            limit: usize::MAX,
            hashed: 0,
        };
        super::body_relaxed(data, &mut hasher);
        hasher.finalize_bytes()
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
        assert_eq!(body_relaxed(b"\r\n"), b"\r\n");
        assert_eq!(body_relaxed(b"hey        \r\n"), b"hey\r\n");
        assert_eq!(body_relaxed(b" C \r\nD \t E\r\n\r\n\r\n"), b" C\r\nD E\r\n");
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
