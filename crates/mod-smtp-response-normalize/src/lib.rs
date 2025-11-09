use chrono::DateTime;
use config::get_or_create_sub_module;
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
    r"^(:?[a-zA-Z0-9+/_\-]{4})+(:?[a-zA-Z0-9+/_\-]{2}==|[a-zA-Z0-9+/_\-]{3}=)?$";

/// Match ipv4 or ipv6 addresses, followed by optional :port.
/// This doesn't do anything about the ipv6 .port syntax.
/// ipv6 portion of this is taken from
/// <https://stackoverflow.com/a/17871737/149111>
const IP_RE: &str = r"^(:?\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}|([0-9a-fA-F]{1,4}:){7,7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]{1,}|::(ffff(:0{1,4}){0,1}:){0,1}((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9])\.){3,3}(25[0-5]|(2[0-4]|1{0,1}[0-9]){0,1}[0-9]))(:?:\d{1,5})?$";

/// Match email addresses.
/// The complicated regex here outperforms the more simplistic and
/// obvious regex that you might otherwise be inclined to write.
/// <https://stackoverflow.com/a/201378/149111>
const EMAIL_RE: &str = r#"^(?:[a-z0-9!#$%&'*+\x2f=?^_`\x7b-\x7d~\x2d]+(?:\.[a-z0-9!#$%&'*+\x2f=?^_`\x7b-\x7d~\x2d]+)*|"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*")@(?:(?:[a-z0-9](?:[a-z0-9\x2d]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9\x2d]*[a-z0-9])?|\[(?:(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9]))\.){3}(?:(2(5[0-5]|[0-4][0-9])|1[0-9][0-9]|[1-9]?[0-9])|[a-z0-9\x2d]*[a-z0-9]:(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21-\x5a\x53-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])+)\])$"#;

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

// Annotated here with bench throughput on my 7965WX.
// Overall is 290 MiB/s 1.4us. Contrast with NOP (empty table)
// throughput of 1.6GiB/s 244ns.
// The numbers next to the entries below are the throughput
// when just that particular item is enabled.
const FUNCS: &[Normalizer] = &[
    // Should always be first
    tokenize_dictionary_word_phf, // 1.4266 GiB/s 272ns
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

    for word in s.split_ascii_whitespace() {
        let word = match normalize_word(word) {
            Some(tokenized) => tokenized,
            None => Cow::Borrowed(word),
        };

        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&word);
    }

    result
}

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
                "Your email has been rate limited because the From: header (RFC5322) in this message isn't aligned with either the authenticated SPF or DKIM organizational domain. To learn more about DMARC alignment, visit https://support.google.com/a?p=dmarc-alignment To learn more about Gmail requirements for bulk senders, visit https://support.google.com/a?p=sender-guidelines. {hash} - gsmtp",
            ),
            (
                "550 5.1.1 The email account that you tried to reach does not exist. Please try double-checking the recipient's email address for typos or unnecessary spaces. For more information, go to  https://support.google.com/mail/?p=NoSuchUser 41be03b00d2f7-b93bf44f0c0si6882731a12.803 - gsmtp",
                "550 5.1.1 The email account that you tried to reach does not exist. Please try double-checking the recipient's email address for typos or unnecessary spaces. For more information, go to https://support.google.com/mail/?p=NoSuchUser {hash} - gsmtp",
            ),
            ("OK ids=8a5475ccbbc611eda12250ebf67f93bd", "OK ids={uuid}"),
        ];

        for (input, expected_output) in CASES {
            let output = normalize(input);

            assert_eq!(output, *expected_output, "input={input}");
        }
    }
}
