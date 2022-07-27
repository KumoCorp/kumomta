// Inspired from https://docs.rs/dkim/latest/src/dkim/canonicalization.rs.html
use crate::bytes;

#[derive(PartialEq, Clone, Debug)]
pub enum Type {
    Simple,
    Relaxed,
}
impl std::string::ToString for Type {
    fn to_string(&self) -> String {
        match self {
            Self::Simple => "simple".to_owned(),
            Self::Relaxed => "relaxed".to_owned(),
        }
    }
}

/// Canonicalize body using the simple canonicalization algorithm.
///
/// The first argument **must** be the body of the mail.
pub(crate) fn canonicalize_body_simple(mut body: &[u8]) -> Vec<u8> {
    if body.is_empty() {
        return b"\r\n".to_vec();
    }

    while body.ends_with(b"\r\n\r\n") {
        body = &body[..body.len() - 2];
    }

    body.to_vec()
}

/// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.3
/// Canonicalize body using the relaxed canonicalization algorithm.  
///
/// The first argument **must** be the body of the mail.
pub(crate) fn canonicalize_body_relaxed(body: &[u8]) -> Vec<u8> {
    let mut body = body.to_vec();
    // See https://tools.ietf.org/html/rfc6376#section-3.4.4 for implementation details

    // Reduce all sequences of WSP within a line to a single SP character.
    bytes::replace(&mut body, '\t', ' ');
    let mut previous = false;
    body.retain(|c| {
        if *c == b' ' {
            if previous {
                false
            } else {
                previous = true;
                true
            }
        } else {
            previous = false;
            true
        }
    });

    // Ignore all whitespace at the end of lines. Implementations MUST NOT remove the CRLF at the end of the line.
    while let Some(idx) = bytes::find(&body, b" \r\n") {
        body.remove(idx);
    }

    // Ignore all empty lines at the end of the message body. "Empty line" is defined in Section 3.4.3.
    while body.ends_with(b"\r\n\r\n") {
        body.remove(body.len() - 1);
        body.remove(body.len() - 1);
    }

    // If the body is non-empty but does not end with a CRLF, a CRLF is added. (For email, this is only possible when using extensions to SMTP or non-SMTP transport mechanisms.)
    if !body.is_empty() && !body.ends_with(b"\r\n") {
        body.push(b'\r');
        body.push(b'\n');
    }

    body
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.1
pub(crate) fn canonicalize_header_simple(key: &str, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&key.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value);
    out.extend_from_slice(b"\r\n");

    out
}

// https://datatracker.ietf.org/doc/html/rfc6376#section-3.4.2
pub(crate) fn canonicalize_header_relaxed(key: &str, value: &[u8]) -> Vec<u8> {
    let key = key.to_lowercase();
    let key = key.trim_end();
    let value = canonicalize_header_value_relaxed(value);

    let mut out = Vec::new();
    out.extend_from_slice(&key.as_bytes());
    out.extend_from_slice(b":");
    out.extend_from_slice(&value);
    out.extend_from_slice(b"\r\n");

    out
}

fn canonicalize_header_value_relaxed(value: &[u8]) -> Vec<u8> {
    let mut value = value.to_vec();
    bytes::replace(&mut value, '\t', ' ');
    value = bytes::replace_slice(&value, b"\r\n", b"");

    while value.ends_with(b" ") {
        value.remove(value.len() - 1);
    }
    while value.starts_with(b" ") {
        value.remove(0);
    }
    let mut previous = false;
    value.retain(|c| {
        if *c == b' ' {
            if previous {
                false
            } else {
                previous = true;
                true
            }
        } else {
            previous = false;
            true
        }
    });

    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonicalize_header_relaxed() {
        assert_eq!(
            canonicalize_header_relaxed("SUBJect", b" AbC\r\n"),
            b"subject:AbC\r\n"
        );
        assert_eq!(
            canonicalize_header_relaxed("Subject \t", b"\t Your Name\t \r\n"),
            b"subject:Your Name\r\n"
        );
        assert_eq!(
            canonicalize_header_relaxed("Subject \t", b"\t Kimi \t \r\n No \t\r\n Na Wa\r\n"),
            b"subject:Kimi No Na Wa\r\n"
        );
    }

    #[test]
    fn test_canonicalize_body_relaxed() {
        assert_eq!(canonicalize_body_relaxed(b"\r\n"), b"\r\n");
        assert_eq!(canonicalize_body_relaxed(b"hey        \r\n"), b"hey\r\n");
    }
}
