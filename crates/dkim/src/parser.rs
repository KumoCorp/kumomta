use crate::{canonicalization, hash, DKIMError};
use nom::bytes::complete::{tag, take_while1};
use nom::character::complete::alpha1;
use nom::combinator::opt;
use nom::multi::fold_many0;
use nom::sequence::{delimited, pair, preceded, terminated};
use nom::{IResult, Parser};

#[derive(Clone, Debug, PartialEq)]
/// DKIM signature tag
pub struct Tag {
    /// Name of the tag (v, i, a, h, ...)
    pub name: String,
    /// Value of the tag with spaces removed
    pub value: String,
    /// Value of the tag as seen in the text
    pub raw_value: String,
}

/// Main entrypoint of the parser. Parses the DKIM signature tag list
/// as specified <https://datatracker.ietf.org/doc/html/rfc6376#section-3.6.1>.
/// tag-list  =  tag-spec *( ";" tag-spec ) [ ";" ]
pub fn tag_list(input: &str) -> IResult<&str, Vec<Tag>> {
    let (input, start) = tag_spec.parse(input)?;

    terminated(
        fold_many0(
            preceded(tag(";"), tag_spec),
            move || vec![start.clone()],
            |mut acc: Vec<Tag>, item| {
                acc.push(item);
                acc
            },
        ),
        opt(tag(";")),
    )
    .parse(input)
}

/// tag-spec  =  [FWS] tag-name [FWS] "=" [FWS] tag-value [FWS]
fn tag_spec(input: &str) -> IResult<&str, Tag> {
    let (input, name) = delimited(opt(fws), tag_name, opt(fws)).parse(input)?;
    let (input, _) = tag("=").parse(input)?;

    // RFC 8616 section 4 allows UTF-8 in the d=, i= and s= tags
    let is_valchar = match name {
        "d" | "i" | "s" => is_valchar_utf8,
        _ => is_valchar,
    };

    // Parse the twice to keep the original text
    let value_input = input;
    let (_, raw_value) =
        delimited(opt(fws), |i| raw_tag_value(i, is_valchar), opt(fws)).parse(value_input)?;
    let (input, value) =
        delimited(opt(fws), |i| tag_value(i, is_valchar), opt(fws)).parse(value_input)?;

    Ok((
        input,
        Tag {
            name: name.to_owned(),
            value,
            raw_value,
        },
    ))
}

/// tag-name  =  ALPHA *ALNUMPUNC
/// ALNUMPUNC =  ALPHA / DIGIT / "_"
fn tag_name(input: &str) -> IResult<&str, &str> {
    alpha1.parse(input)
}

/// VALCHAR   =  %x21-3A / %x3C-7E
fn is_valchar(c: char) -> bool {
    ('!'..=':').contains(&c) || ('<'..='~').contains(&c)
}

/// VALCHAR extended with UTF-8 per RFC 8616 section 4,
/// excluding Unicode whitespace and control characters
fn is_valchar_utf8(c: char) -> bool {
    is_valchar(c) || (!c.is_ascii() && !c.is_whitespace() && !c.is_control())
}

/// tag-value =  [ tval *( 1*(WSP / FWS) tval ) ]
/// tval      =  1*VALCHAR
fn tag_value(input: &str, is_valchar: fn(char) -> bool) -> IResult<&str, String> {
    match opt(take_while1(is_valchar)).parse(input)? {
        (input, Some(start)) => fold_many0(
            preceded(fws, take_while1(is_valchar)),
            || start.to_owned(),
            |mut acc: String, item| {
                acc += item;
                acc
            },
        )
        .parse(input),
        (input, None) => Ok((input, "".to_string())),
    }
}

fn raw_tag_value(input: &str, is_valchar: fn(char) -> bool) -> IResult<&str, String> {
    match opt(take_while1(is_valchar)).parse(input)? {
        (input, Some(start)) => fold_many0(
            pair(fws, take_while1(is_valchar)),
            || start.to_owned(),
            |mut acc: String, item| {
                acc += &(item.0.to_owned() + item.1);
                acc
            },
        )
        .parse(input),
        (input, None) => Ok((input, "".to_string())),
    }
}

/// FWS is folding whitespace.  It allows multiple lines separated by CRLF followed by at least one whitespace, to be joined.
fn fws(input: &str) -> IResult<&str, &str> {
    take_while1(|c| c == ' ' || c == '\t' || c == '\r' || c == '\n').parse(input)
}

pub(crate) fn parse_hash_algo(value: &str) -> Result<hash::HashAlgo, DKIMError> {
    use hash::HashAlgo;
    match value {
        "rsa-sha1" => Ok(HashAlgo::RsaSha1),
        "rsa-sha256" => Ok(HashAlgo::RsaSha256),
        "ed25519-sha256" => Ok(HashAlgo::Ed25519Sha256),
        e => Err(DKIMError::UnsupportedHashAlgorithm(e.to_string())),
    }
}

/// Parses the canonicalization value (passed in c=) and returns canonicalization
/// for (Header, Body)
pub(crate) fn parse_canonicalization(
    value: Option<&str>,
) -> Result<(canonicalization::Type, canonicalization::Type), DKIMError> {
    use canonicalization::Type::{Relaxed, Simple};
    match value {
        None => Ok((Simple, Simple)),
        Some(s) => match s {
            "simple/simple" => Ok((Simple, Simple)),
            "relaxed/simple" => Ok((Relaxed, Simple)),
            "simple/relaxed" => Ok((Simple, Relaxed)),
            "relaxed/relaxed" => Ok((Relaxed, Relaxed)),
            "relaxed" => Ok((Relaxed, Simple)),
            "simple" => Ok((Simple, Simple)),
            v => Err(DKIMError::UnsupportedCanonicalizationType(v.to_owned())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonicalization_empty() {
        use canonicalization::Type::Simple;
        assert_eq!(parse_canonicalization(None).unwrap(), (Simple, Simple));
    }

    #[test]
    fn test_canonicalization_one_algo() {
        use canonicalization::Type::{Relaxed, Simple};

        assert_eq!(
            parse_canonicalization(Some("simple")).unwrap(),
            (Simple, Simple)
        );
        assert_eq!(
            parse_canonicalization(Some("relaxed")).unwrap(),
            (Relaxed, Simple)
        );
    }

    #[test]
    fn test_tag_list() {
        assert_eq!(
            tag_list("a = a/1@.-:= ").unwrap(),
            (
                "",
                vec![Tag {
                    name: "a".to_string(),
                    value: "a/1@.-:=".to_string(),
                    raw_value: "a/1@.-:=".to_string()
                }]
            )
        );
        assert_eq!(
            tag_list("a= a ; b = a\n    bc").unwrap(),
            (
                "",
                vec![
                    Tag {
                        name: "a".to_string(),
                        value: "a".to_string(),
                        raw_value: "a".to_string()
                    },
                    Tag {
                        name: "b".to_string(),
                        value: "abc".to_string(),
                        raw_value: "a\n    bc".to_string()
                    }
                ]
            )
        );
    }

    #[test]
    fn test_tag_list_utf8() {
        let (rest, tags) = tag_list("d=日本語.jp; s=セレクタ; i=ユーザー@mail.日本語.jp").unwrap();
        assert_eq!(rest, "");
        let values: Vec<_> = tags
            .iter()
            .map(|t| (t.name.as_str(), t.value.as_str()))
            .collect();
        assert_eq!(
            values,
            [
                ("d", "日本語.jp"),
                ("s", "セレクタ"),
                ("i", "ユーザー@mail.日本語.jp")
            ]
        );

        // UTF-8 is only permitted in d=, i= and s=
        let (rest, tags) = tag_list("a=日本").unwrap();
        assert_eq!(rest, "日本");
        assert_eq!(tags[0].value, "");

        // Unicode whitespace (here U+3000 ideographic space) ends the value
        let (rest, tags) = tag_list("d=日本語\u{3000}.jp").unwrap();
        assert_eq!(rest, "\u{3000}.jp");
        assert_eq!(tags[0].value, "日本語");
    }

    #[test]
    fn test_tag_spec() {
        assert_eq!(
            tag_spec("a=b").unwrap(),
            (
                "",
                Tag {
                    name: "a".to_string(),
                    value: "b".to_string(),
                    raw_value: "b".to_string()
                }
            )
        );
        assert_eq!(
            tag_spec("a=b c d e f").unwrap(),
            (
                "",
                Tag {
                    name: "a".to_string(),
                    value: "bcdef".to_string(),
                    raw_value: "b c d e f".to_string()
                }
            )
        );
    }

    #[test]
    fn test_tag_list_dns() {
        assert_eq!(
            tag_list("k=rsa; p=kEy+/").unwrap(),
            (
                "",
                vec![
                    Tag {
                        name: "k".to_string(),
                        value: "rsa".to_string(),
                        raw_value: "rsa".to_string()
                    },
                    Tag {
                        name: "p".to_string(),
                        value: "kEy+/".to_string(),
                        raw_value: "kEy+/".to_string()
                    }
                ]
            )
        );
    }
}
