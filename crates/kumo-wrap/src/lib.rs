pub fn wrap(value: &str) -> String {
    const SOFT_WIDTH: usize = 75;
    const HARD_WIDTH: usize = 900;
    wrap_impl(value, SOFT_WIDTH, HARD_WIDTH)
}

/// We can't use textwrap::fill here because it will prefer to break
/// a line rather than finding stuff that fits.  We use a simple
/// algorithm that tries to fill up to the desired width, allowing
/// for overflow if there is a word that is too long to fit in
/// the header, but breaking after a hard limit threshold.
pub fn wrap_impl(value: &str, soft_width: usize, hard_width: usize) -> String {
    let mut result = String::new();
    let mut line = String::new();

    for word in value.split_ascii_whitespace() {
        if line.len() + word.len() < soft_width {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
            continue;
        }

        // Need to wrap.

        // Accumulate line so far, if any
        if !line.is_empty() {
            if !result.is_empty() {
                // There's an existing line, start a new one, indented
                result.push('\t');
            }
            result.push_str(&line);
            result.push_str("\r\n");
            line.clear();
        }

        // build out a line from the characters of this one
        if word.len() <= hard_width {
            line.push_str(word);
        } else {
            for c in word.chars() {
                line.push(c);
                if line.len() >= hard_width {
                    if !result.is_empty() {
                        result.push('\t');
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
            result.push('\t');
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
                expect,
                "input: '{input}' should produce '{expect}'"
            );
        }
    }
}
