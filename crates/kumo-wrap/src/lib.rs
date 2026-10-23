use bstr::{BStr, ByteVec};

pub const SOFT_WIDTH: usize = 75;
pub const HARD_WIDTH: usize = 900;

pub fn wrap(value: &str) -> String {
    String::from_utf8(wrap_impl(value, SOFT_WIDTH, HARD_WIDTH)).expect("utf8-in, utf8-out")
}

pub fn wrap_bytes(value: impl AsRef<BStr>) -> Vec<u8> {
    wrap_impl(value, SOFT_WIDTH, HARD_WIDTH)
}

/// We can't use textwrap::fill here because it will prefer to break
/// a line rather than finding stuff that fits.  We use a simple
/// algorithm that tries to fill up to the desired width, allowing
/// for overflow if there is a word that is too long to fit in
/// the header, but breaking after a hard limit threshold.
pub fn wrap_impl(value: impl AsRef<BStr>, soft_width: usize, hard_width: usize) -> Vec<u8> {
    let value: &BStr = value.as_ref();
    let mut result: Vec<u8> = vec![];
    let mut line: Vec<u8> = vec![];

    for word in value.split(|&b| b.is_ascii_whitespace()) {
        if word.is_empty() {
            continue;
        }
        if line.len() + word.len() < soft_width {
            if !line.is_empty() {
                line.push(b' ');
            }
            line.push_str(word);
            continue;
        }

        // Need to wrap.

        // Accumulate line so far, if any
        if !line.is_empty() {
            if !result.is_empty() {
                // There's an existing line, start a new one, indented
                result.push(b'\t');
            }
            result.push_str(&line);
            result.push_str("\r\n");
            line.clear();
        }

        // build out a line from the characters of this word. `word` may contain
        // multi-byte UTF-8 sequences (eg. an RFC 6531 addr-spec, which is
        // emitted as raw UTF-8 rather than encoded-word wrapped), so
        // hard-wrapping must cut on char boundaries to avoid producing invalid
        // UTF-8. `word` isn't guaranteed to be valid UTF-8 (eg. it may come
        // from an 8-bit header value), so we walk it in utf8_chunks and only
        // split within the valid stretches. Any invalid byte run is pushed
        // through unchanged rather than lossily replaced.
        if word.len() <= hard_width {
            line.push_str(word);
        } else {
            for chunk in word.utf8_chunks() {
                for c in chunk.valid().chars() {
                    let mut buf = [0u8; 4];
                    line.push_str(c.encode_utf8(&mut buf).as_bytes());
                    if line.len() >= hard_width {
                        if !result.is_empty() {
                            result.push(b'\t');
                        }
                        result.push_str(&line);
                        result.push_str("\r\n");
                        line.clear();
                    }
                }
                if !chunk.invalid().is_empty() {
                    line.push_str(chunk.invalid());
                    if line.len() >= hard_width {
                        if !result.is_empty() {
                            result.push(b'\t');
                        }
                        result.push_str(&line);
                        result.push_str("\r\n");
                        line.clear();
                    }
                }
            }
        }
    }

    if !line.is_empty() {
        if !result.is_empty() {
            result.push(b'\t');
        }
        result.push_str(&line);
    }

    result
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn wrapping() {
        for (input, expect) in [
            ("foo", "foo"),
            ("hi there", "hi there"),
            ("hello world", "hello\r\n\tworld"),
            ("hello world ", "hello\r\n\tworld"),
            (
                "hello world foo bar baz woot woot",
                "hello\r\n\tworld foo\r\n\tbar baz\r\n\twoot woot",
            ),
            (
                "hi there breakmepleaseIamtoolong",
                "hi there\r\n\tbreakmepleaseIa\r\n\tmtoolong",
            ),
        ] {
            let wrapped = wrap_impl(input, 10, 15);
            k9::assert_equal!(
                wrapped,
                expect.as_bytes(),
                "input: '{input}' should produce '{expect}'"
            );
        }
    }

    /// A multi-byte word past the hard limit must split on char boundaries.
    /// The 4-byte char with a hard limit of 15 (not a multiple of 4) forces
    /// the wrap point inside a character, where byte-by-byte wrapping would
    /// emit invalid UTF-8; the from_utf8 below is the check that catches it.
    #[test]
    fn hard_wrap_multibyte_word() {
        let word = "😀".repeat(10); // 40 bytes, no ascii whitespace to fold at
        let wrapped = wrap_impl(word.as_str(), 10, 15);
        let text = String::from_utf8(wrapped).expect("wrapped output is valid UTF-8");
        k9::assert_equal!(text.replace(['\r', '\n', '\t'], ""), word);
    }

    /// wrap() validates its output as UTF-8 via expect(); a word past the hard
    /// limit whose characters do not align to it (the leading ASCII byte
    /// offsets them) would, without char-boundary splitting, make that
    /// validation panic.
    #[test]
    fn wrap_does_not_panic_on_long_multibyte_word() {
        let word = format!("x{}", "😀".repeat(300)); // 1 + 1200 bytes, misaligned
        let wrapped = wrap(&word);
        k9::assert_equal!(wrapped.replace(['\r', '\n', '\t'], ""), word);
    }

    /// An over-long word carrying invalid UTF-8 keeps those bytes verbatim:
    /// the hard-wrap walks utf8_chunks and must emit each invalid run as-is
    /// rather than dropping it or substituting the replacement character.
    /// Reachable only via wrap_bytes, since wrap() requires valid UTF-8.
    #[test]
    fn hard_wrap_preserves_invalid_utf8() {
        let mut word = b"abc".to_vec();
        word.extend_from_slice(&[0xff, 0xfe]); // invalid UTF-8 run
        word.extend_from_slice(b"defghijklmnopqrstuv"); // pad past the hard limit
        let wrapped = wrap_impl(word.as_slice(), 10, 15);
        // Removing the inserted fold bytes must reconstruct the input exactly,
        // invalid bytes included; the input has no CR/LF/TAB of its own.
        let stripped: Vec<u8> = wrapped
            .into_iter()
            .filter(|b| !matches!(b, b'\r' | b'\n' | b'\t'))
            .collect();
        k9::assert_equal!(stripped, word);
    }
}
