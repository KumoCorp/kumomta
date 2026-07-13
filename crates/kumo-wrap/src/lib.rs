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
        if word.len() == 0 {
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

        // build out a line from the characters of this one
        if word.len() <= hard_width {
            line.push_str(word);
        } else {
            for &c in word.iter() {
                line.push(c);
                if line.len() >= hard_width {
                    if !result.is_empty() {
                        result.push(b'\t');
                    }
                    result.push_str(&line);
                    result.push_str("\r\n");
                    line.clear();
                    continue;
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
}
