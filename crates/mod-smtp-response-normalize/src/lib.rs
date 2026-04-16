use chrono::DateTime;
#[cfg(feature = "lua")]
use config::get_or_create_sub_module;
#[cfg(feature = "lua")]
use mlua::Lua;
use regex::{RegexSet, RegexSetBuilder};
use std::borrow::Cow;
use std::sync::LazyLock;
use uuid::Uuid;
mod dict;

type Normalizer = for<'a> fn(word: &'a str) -> Option<Cow<'a, str>>;

fn tokenize_timestamp_3339<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    DateTime::parse_from_rfc3339(word)
        .ok()
        .map(|_| Cow::Borrowed("{timestamp}"))
}

fn tokenize_uuid<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    Uuid::try_parse(word).ok().map(|_| Cow::Borrowed("{uuid}"))
}

/// A number of dictionary words are technically valid base64 (eg: "duration")
/// and we don't want them to be flagged as base64.  This recognizes
/// a dictionary word and returns that word as the token, preventing
/// further tokenization
fn tokenize_dictionary_word_phf<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    if crate::dict::DICT.contains(word) {
        Some(Cow::Borrowed(word))
    } else {
        None
    }
}

/// Match either base64 or base64-url
const BASE64_RE: &str =
    r"^(?:[a-zA-Z0-9+/_\-]{4})+(?:[a-zA-Z0-9+/_\-]{2}==|[a-zA-Z0-9+/_\-]{3}=)?$";

/// Match ipv4 or ipv6 addresses, followed by optional :port.
/// This doesn't do anything about the ipv6 .port syntax.
/// ipv6 portion of this is taken from
/// <https://stackoverflow.com/a/17871737/149111>
const IP_RE: &str = r"^(?:\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}|(?:[0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|(?:[0-9a-fA-F]{1,4}:){1,7}:|(?:[0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|(?:[0-9a-fA-F]{1,4}:){1,5}(?::[0-9a-fA-F]{1,4}){1,2}|(?:[0-9a-fA-F]{1,4}:){1,4}(?::[0-9a-fA-F]{1,4}){1,3}|(?:[0-9a-fA-F]{1,4}:){1,3}(?::[0-9a-fA-F]{1,4}){1,4}|(?:[0-9a-fA-F]{1,4}:){1,2}(?::[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:(?:(?::[0-9a-fA-F]{1,4}){1,6})|:(?:(?::[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(?::[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(?:ffff(?::0{1,4}){0,1}:){0,1}(?:(?:25[0-5]|(?:2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(?:25[0-5]|(?:2[0-4]|1{0,1}[0-9]){0,1}[0-9])|(?:[0-9a-fA-F]{1,4}:){1,4}:(?:(?:25[0-5]|(?:2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(?:25[0-5]|(?:2[0-4]|1{0,1}[0-9]){0,1}[0-9]))(:?:\d{1,5})?$";

/// Match email addresses.
/// The complicated regex here outperforms the more simplistic and
/// obvious regex that you might otherwise be inclined to write.
/// <https://stackoverflow.com/a/201378/149111>
const EMAIL_RE: &str = r#"^(?:[a-z0-9!#$%&'*+\x2f=?^_`\x7b-\x7d~\x2d]+(?:\.[a-z0-9!#$%&'*+\x2f=?^_`\x7b-\x7d~\x2d]+)*|"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*")@(?:(?:[a-z0-9](?:[a-z0-9\x2d]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9\x2d]*[a-z0-9])?|\[(?:(?:(?:2(?:5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9]))\.){3}(?:(?:2(?:5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9])|[a-z0-9\x2d]*[a-z0-9]:(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21-\x5a\x53-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])+)\])$"#;

/// Match ISO 8601 duration strings.
/// Format: P[n]Y[n]M[n]DT[n]H[n]M[n]S where P is the duration designator,
/// T separates date and time components, and each component is optional.
/// Examples: "P23DT23H", "P4Y", "P1Y2M3DT4H5M6S"
const ISO8601_DURATION_RE: &str = r"^P(?:\d+(?:\.\d+)?Y)?(?:\d+(?:\.\d+)?M)?(?:\d+(?:\.\d+)?W)?(?:\d+(?:\.\d+)?D)?(?:T(?:\d+(?:\.\d+)?H)?(?:\d+(?:\.\d+)?M)?(?:\d+(?:\.\d+)?S)?)?$";

fn tokenize_re<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    static MAPPING: &[(&str, &str)] = &[
        (IP_RE, "{ipaddr}"),
        (BASE64_RE, "{base64}"),
        (EMAIL_RE, "{email}"),
    ];
    static SET: LazyLock<RegexSet> = LazyLock::new(|| {
        RegexSetBuilder::new(MAPPING.iter().map(|(re, _label)| re))
            .build()
            .unwrap()
    });

    let matching_idx: usize = SET.matches(word).into_iter().next()?;

    Some(Cow::Borrowed(MAPPING[matching_idx].1))
}

/// Tokenize things that look a bit like some kind of hash that are
/// not otherwise matchable as base64 or a uuid.
/// We only consider words that are 8 or more characters and we
/// want to see a mix of alphanumerics and punctuation like `-._`.
/// We need at least two alpha and two numeric characters to
/// consider it hashy enough.
/// You might wonder why we can't encode this as a regex; the answer
/// is that the regex crate doesn't support the lookaround assertions
/// required to prevent this from matching dictionary words or simple
/// numbers, and the fancy-regex crate, which does support those
/// assertions, doesn't support the regex set builder.
fn tokenize_hash<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    if word.len() < 8 {
        return None;
    }

    let mut num_alpha = 0;
    let mut num_digit = 0;

    for c in word.chars() {
        if c.is_ascii_alphabetic() {
            num_alpha += 1;
        } else if c.is_ascii_digit() {
            num_digit += 1;
        } else if c == '-' || c == '.' || c == '_' {
            // OK
        } else {
            // Not hash-y
            return None;
        }
    }

    if num_alpha > 2 && num_digit > 2 {
        return Some(Cow::Borrowed("{hash}"));
    }

    None
}

/// Look for `something=token` and replace the RHS with a token.
/// This recurses on the RHS of the equals sign.
fn tokenize_compound<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    if let Some((lhs, rhs)) = word.split_once('=') {
        let tokenized = normalize_word(rhs)?;
        Some(format!("{lhs}={tokenized}").into())
    } else {
        None
    }
}

/// Preprocess the input string to replace duration strings with a placeholder.
/// Duration strings are sequences like "11s 999ms 990us 55ns".
/// This must happen before splitting by whitespace since the duration itself
/// contains whitespace.
/// We use a placeholder without special characters so the bracket processing doesn't
/// strip braces from it.
fn preprocess_duration<'a>(s: &'a str) -> Cow<'a, str> {
    // Pattern: number followed by time unit, optionally repeated with whitespace
    // Units: ns, us, ms, s, m, h, day, month, year
    // This regex matches sequences like "11s 999ms 990us 55ns"
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        // Match: number + unit, optionally followed by (whitespace + number + unit) repeated
        // Example: "11s 999ms 990us 55ns"
        // Word boundaries ensure we don't match "70s" inside "70si756..." or "abc11s"
        let pattern =
            r"\b\d+(?:ns|us|ms|s|m|h|day|month|year)(?:\s+\d+(?:ns|us|ms|s|m|h|day|month|year))*\b";
        regex::Regex::new(pattern).unwrap()
    });

    // Replace all duration matches with a simple placeholder
    let result = RE.replace_all(s, "__DURATION__");
    Cow::Owned(result.into_owned())
}

/// Tokenize duration placeholders like "__DURATION__" to {duration}
/// Also recognizes ISO 8601 duration strings (e.g., "P23DT23H", "P4Y", "P1Y2M3DT4H5M6S")
fn tokenize_duration<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    if word == "__DURATION__" {
        Some(Cow::Borrowed("{duration}"))
    } else if word.starts_with('P') {
        // ISO 8601 duration format: P[n]Y[n]M[n]DT[n]H[n]M[n]S
        // T separates date and time components
        // Check if it matches the ISO 8601 pattern
        static ISO8601_RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(ISO8601_DURATION_RE).unwrap());
        if ISO8601_RE.is_match(word) {
            Some(Cow::Borrowed("{duration}"))
        } else {
            None
        }
    } else {
        None
    }
}

// Annotated here with bench throughput on my 7965WX.
// Overall is 290 MiB/s 1.4us. Contrast with NOP (empty table)
// throughput of 1.6GiB/s 244ns.
// The numbers next to the entries below are the throughput
// when just that particular item is enabled.
const FUNCS: &[Normalizer] = &[
    // Should always be first
    tokenize_dictionary_word_phf, // 1.4266 GiB/s 272ns
    tokenize_duration,            // duration placeholder
    tokenize_timestamp_3339,      // 1017MiB/s 392ns
    tokenize_uuid,                // 1.13GiB/s 342ns
    tokenize_hash,                // 1.19GiB/s 325ns
    tokenize_re,                  // 233MiB/s 1.7us
    // Should always be last
    tokenize_compound,
];

fn normalize_word<'a>(word: &'a str) -> Option<Cow<'a, str>> {
    for func in FUNCS {
        let res = (func)(word);
        if res.is_some() {
            return res;
        }
    }
    None
}

pub fn normalize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());

    // Preprocess to replace duration strings before splitting
    // We need to do this first because duration strings contain whitespace
    let s = match preprocess_duration(s) {
        Cow::Borrowed(b) => b.to_string(),
        Cow::Owned(o) => o,
    };

    let mut processed;

    // pre-process to remove parenthetical delimited sequences and replace
    // that punctuation with whitespace.  That allows the tokenizer to
    // see more tokens and do a better job, without harming the prose
    // in the response text.
    // Do a quick test to see if any opening parens are present so that
    // we can avoid allocating an additional string in the more common case
    // where they are not present.
    //
    // This transforms eg: " [" -> " " and "] " -> " ",
    // for each ASCII bracket character.
    //
    // To spell that out a bit more clearly, this transformation has the
    // side effect of changing " (RFC5322) " into " RFC5322 "
    // in the normalized output.
    let needs_process = memchr::memchr3(b'[', b'(', b'{', s.as_bytes()).is_some();
    let s = if needs_process {
        processed = String::with_capacity(s.len());
        let mut iter = s.chars().peekable();
        while let Some(c) = iter.next() {
            if (c.is_ascii_whitespace() || processed.is_empty())
                && matches!(iter.peek(), Some('[' | '(' | '{'))
            {
                iter.next();
                processed.push(' ');
                continue;
            }

            if matches!(c, ']' | ')' | '}')
                && iter
                    .peek()
                    .map(|c| c.is_ascii_whitespace() || c.is_ascii_punctuation())
                    .unwrap_or(true)
            {
                iter.next();
                processed.push(' ');
                continue;
            }

            processed.push(c);
        }
        &processed
    } else {
        &s
    };

    for word in s.split_ascii_whitespace() {
        let word = match normalize_word(word) {
            Some(tokenized) => tokenized,
            None => Cow::Borrowed(word),
        };

        // Collapse runs of 1+ spaces (implied between the split iter)
        // into a single space character
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&word);
    }

    result
}

#[cfg(feature = "lua")]
pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let string_mod = get_or_create_sub_module(lua, "string")?;

    string_mod.set(
        "normalize_smtp_response",
        lua.create_function(move |_, text: String| Ok(normalize(&text)))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn various() {
        const CASES: &[(&str, &str)] = &[
            (
                "retry again at 2025-11-06T17:11:34.261306612Z",
                "retry again at {timestamp}",
            ),
            (
                "a uuid 10aa5da5-3f3b-4176-beb9-32875830f082",
                "a uuid {uuid}",
            ),
            ("aGVsbG8uCg==", "{base64}"),
            ("aGVsbG8K", "{base64}"),
            ("aGVsbG8K aGVsbG8K", "{base64} {base64}"),
            ("hello aGVsbG8uCg==", "hello {base64}"),
            ("hello", "hello"),
            ("hello aGVsbG8K", "hello {base64}"),
            (
                "421 4.1.0 10.0.0.1 throttled try later",
                "421 4.1.0 {ipaddr} throttled try later",
            ),
            (
                "421 4.1.0 ::1 throttled try later",
                "421 4.1.0 {ipaddr} throttled try later",
            ),
            (
                "Accepting connection from 42.69.10.20:25",
                "Accepting connection from {ipaddr}",
            ),
            ("duration 00:10:34", "duration 00:10:34"),
            (
                "rejecting mail for some.body@gmail.com",
                "rejecting mail for {email}",
            ),
            (
                "Your email has been rate limited because the From: header (RFC5322) in this message isn't aligned with either the authenticated SPF or DKIM organizational domain. To learn more about DMARC alignment, visit  https://support.google.com/a?p=dmarc-alignment  To learn more about Gmail requirements for bulk senders, visit  https://support.google.com/a?p=sender-guidelines. a640c23a62f3a-ab67626ed70si756442266b.465 - gsmtp",
                "Your email has been rate limited because the From: header RFC5322 in this message isn't aligned with either the authenticated SPF or DKIM organizational domain. To learn more about DMARC alignment, visit https://support.google.com/a?p=dmarc-alignment To learn more about Gmail requirements for bulk senders, visit https://support.google.com/a?p=sender-guidelines. {hash} - gsmtp",
            ),
            (
                "550 5.1.1 The email account that you tried to reach does not exist. Please try double-checking the recipient's email address for typos or unnecessary spaces. For more information, go to  https://support.google.com/mail/?p=NoSuchUser 41be03b00d2f7-b93bf44f0c0si6882731a12.803 - gsmtp",
                "550 5.1.1 The email account that you tried to reach does not exist. Please try double-checking the recipient's email address for typos or unnecessary spaces. For more information, go to https://support.google.com/mail/?p=NoSuchUser {hash} - gsmtp",
            ),
            ("OK ids=8a5475ccbbc611eda12250ebf67f93bd", "OK ids={uuid}"),
            (
                "550 Mail is rejected by recipients [aGVsbG8uCg== IP: 10.10.10.10]. https://service.mail.qq.com/detail/0/92.",
                "550 Mail is rejected by recipients {base64} IP: {ipaddr} https://service.mail.qq.com/detail/0/92.",
            ),
            (
                "Context: DispatcherDrop. Next due in 11s 999ms 990us 55ns at 2026-04-05T07:34:04.198063031Z",
                "Context: DispatcherDrop. Next due in {duration} at {timestamp}",
            ),
            ("P23DT23H", "{duration}"),
            ("P4Y", "{duration}"),
            ("P1Y2M3DT4H5M6S", "{duration}"),
            ("P1Y2M3DT4H5M6Shello", "{hash}"),
            ("abc11s 999ms", "abc11s {duration}"),
            ("2year", "{duration}"),
            ("1month", "{duration}"),
            ("3day", "{duration}"),
            ("5h 30m", "{duration}"),
            ("2yearhello", "2yearhello"),
        ];

        for (input, expected_output) in CASES {
            let output = normalize(input);

            k9::assert_equal!(output, *expected_output, "input={input}");
        }
    }
}
