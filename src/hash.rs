use std::collections::HashMap;

use base64::engine::general_purpose;
use base64::Engine;
use slog::debug;

use crate::canonicalization::{
    self, canonicalize_body_relaxed, canonicalize_body_simple, canonicalize_header_relaxed,
    canonicalize_header_simple,
};
use crate::header::HEADER;
use crate::{bytes, DKIMError, DKIMHeader};

#[derive(Debug, Clone)]
pub enum HashAlgo {
    RsaSha1,
    RsaSha256,
    Ed25519Sha256,
}

/// Get the body part of an email
fn get_body<'a>(email: &'a mailparse::ParsedMail<'a>) -> Result<&'a [u8], DKIMError> {
    Ok(bytes::get_all_after(email.raw_bytes, b"\r\n\r\n"))
}

fn hash_sha1<T: AsRef<[u8]>>(data: T) -> Vec<u8> {
    use sha1::{Digest, Sha1};

    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn hash_sha256<T: AsRef<[u8]>>(data: T) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Returns the hash of message's body
/// https://datatracker.ietf.org/doc/html/rfc6376#section-3.7
pub(crate) fn compute_body_hash<'a>(
    canonicalization_type: canonicalization::Type,
    length: Option<String>,
    hash_algo: HashAlgo,
    email: &'a mailparse::ParsedMail<'a>,
) -> Result<String, DKIMError> {
    let body = get_body(email)?;

    let mut canonicalized_body = if canonicalization_type == canonicalization::Type::Simple {
        canonicalize_body_simple(&body)
    } else {
        canonicalize_body_relaxed(&body)
    };
    if let Some(length) = length {
        let length = length
            .parse::<usize>()
            .map_err(|err| DKIMError::SignatureSyntaxError(format!("invalid length: {}", err)))?;
        canonicalized_body.truncate(length);
    };

    let hash = match hash_algo {
        HashAlgo::RsaSha1 => hash_sha1(&canonicalized_body),
        HashAlgo::RsaSha256 => hash_sha256(&canonicalized_body),
        HashAlgo::Ed25519Sha256 => hash_sha256(&canonicalized_body),
    };
    Ok(general_purpose::STANDARD.encode(hash))
}

fn select_headers<'a>(
    dkim_header: &str,
    email: &'a mailparse::ParsedMail<'a>,
) -> Result<Vec<(String, &'a [u8])>, DKIMError> {
    let mut signed_headers = vec![];

    let email_headers = &email.headers;
    let num_headers = email_headers.len();
    let mut last_index: HashMap<String, usize> = HashMap::new();

    'outer: for name in dkim_header
        .split(':')
        .map(|h| h.trim().to_ascii_lowercase())
    {
        let index = last_index.get(&name).unwrap_or(&num_headers);
        for header in email_headers
            .iter()
            .enumerate()
            .rev()
            .skip(num_headers - index)
        {
            if header.1.get_key_ref().eq_ignore_ascii_case(&name) {
                signed_headers.push((header.1.get_key(), header.1.get_value_raw()));
                last_index.insert(name, header.0);
                continue 'outer;
            }
        }

        last_index.insert(name, 0);
    }

    Ok(signed_headers)
}

pub(crate) fn compute_headers_hash<'a, 'b>(
    logger: Option<&slog::Logger>,
    canonicalization_type: canonicalization::Type,
    headers: &'b str,
    hash_algo: HashAlgo,
    dkim_header: &'b DKIMHeader,
    email: &'a mailparse::ParsedMail<'a>,
) -> Result<Vec<u8>, DKIMError> {
    let mut input = Vec::new();

    // Add the headers defined in `h=` in the hash
    for (key, value) in select_headers(headers, email)? {
        let canonicalized_value = if canonicalization_type == canonicalization::Type::Simple {
            canonicalize_header_simple(&key, value)
        } else {
            canonicalize_header_relaxed(&key, value)
        };
        input.extend_from_slice(&canonicalized_value);
    }

    // Add the DKIM-Signature header in the hash. Remove the value of the
    // signature (b) first.
    {
        let sign = dkim_header.get_raw_tag("b").unwrap();
        let value = dkim_header.raw_bytes.replace(&sign, "");
        let mut canonicalized_value = if canonicalization_type == canonicalization::Type::Simple {
            canonicalize_header_simple(HEADER, value.as_bytes())
        } else {
            canonicalize_header_relaxed(HEADER, value.as_bytes())
        };

        // remove trailing "\r\n"
        canonicalized_value.truncate(canonicalized_value.len() - 2);

        input.extend_from_slice(&canonicalized_value);
    }
    if let Some(logger) = logger {
        debug!(logger, "headers to hash: {:?}", input);
    }

    let hash = match hash_algo {
        HashAlgo::RsaSha1 => hash_sha1(&input),
        HashAlgo::RsaSha256 => hash_sha256(&input),
        HashAlgo::Ed25519Sha256 => hash_sha256(&input),
    };
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dkim_header() -> DKIMHeader {
        crate::validate_header("v=1; a=rsa-sha256; q=dns/txt; c=relaxed/relaxed; s=smtp; d=test.com; t=1641506955; h=content-type:to: subject:date:from:mime-version:sender; bh=PU2XIErWsXvhvt1W96ntPWZ2VImjVZ3vBY2T/A+wA3A=; b=PIO0A014nyntOGKdTdtvCJor9ZxvP1M3hoLeEh8HqZ+RvAyEKdAc7VOg+/g/OTaZgsmw6U sZCoN0YNVp+2o9nkaeUslsVz3M4I55HcZnarxl+fhplIMcJ/3s0nIhXL51MfGPRqPbB7/M Gjg9/07/2vFoid6Kitg6Z+CfoD2wlSRa8xDfmeyA2cHpeVuGQhGxu7BXuU8kGbeM4+weit Ql3t9zalhikEPI5Pr7dzYFrgWNOEO6w6rQfG7niKON1BimjdbJlGanC7cO4UL361hhXT4X iXLnC9TG39xKFPT/+4nkHy8pp6YvWkD3wKlBjwkYNm0JvKGwTskCMDeTwxXhAg==").unwrap()
    }

    #[test]
    fn test_compute_body_hash_simple() {
        let email = mailparse::parse_mail(
            r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
            .as_bytes(),
        )
        .unwrap();

        let canonicalization_type = canonicalization::Type::Simple;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(
                canonicalization_type.clone(),
                length.clone(),
                hash_algo,
                &email
            )
            .unwrap(),
            "uoq1oCgLlTqpdDX/iUbLy7J1Wic="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "frcCV1k9oG9oKj3dpUqdJg1PxRT2RSN/XKdLCPjaYaY="
        )
    }

    #[test]
    fn test_compute_body_hash_relaxed() {
        let email = mailparse::parse_mail(
            r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
            .as_bytes(),
        )
        .unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(
                canonicalization_type.clone(),
                length.clone(),
                hash_algo,
                &email
            )
            .unwrap(),
            "2jmj7l5rSw0yVb/vlWAYkK/YBwk="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        )
    }

    #[test]
    fn test_compute_body_hash_length() {
        let email = mailparse::parse_mail(
            r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
            .as_bytes(),
        )
        .unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let length = Some("3".to_owned());
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(
                canonicalization_type.clone(),
                length.clone(),
                hash_algo,
                &email
            )
            .unwrap(),
            "2jmj7l5rSw0yVb/vlWAYkK/YBwk="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        )
    }

    #[test]
    fn test_compute_body_hash_empty_simple() {
        let email = mailparse::parse_mail(&[]).unwrap();

        let canonicalization_type = canonicalization::Type::Simple;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(
                canonicalization_type.clone(),
                length.clone(),
                hash_algo,
                &email
            )
            .unwrap(),
            "uoq1oCgLlTqpdDX/iUbLy7J1Wic="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "frcCV1k9oG9oKj3dpUqdJg1PxRT2RSN/XKdLCPjaYaY="
        )
    }

    #[test]
    fn test_compute_body_hash_empty_relaxed() {
        let email = mailparse::parse_mail(&[]).unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(
                canonicalization_type.clone(),
                length.clone(),
                hash_algo,
                &email
            )
            .unwrap(),
            "2jmj7l5rSw0yVb/vlWAYkK/YBwk="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        )
    }

    #[test]
    fn test_compute_headers_hash_simple() {
        let email = mailparse::parse_mail(
            r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
            .as_bytes(),
        )
        .unwrap();

        let canonicalization_type = canonicalization::Type::Simple;
        let hash_algo = HashAlgo::RsaSha1;
        let headers = "To: Subject".to_owned();
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        assert_eq!(
            compute_headers_hash(
                Some(&logger),
                canonicalization_type.clone(),
                &headers,
                hash_algo,
                &dkim_header(),
                &email
            )
            .unwrap(),
            &[
                214, 155, 167, 0, 209, 70, 127, 126, 160, 53, 79, 106, 141, 240, 35, 121, 255, 190,
                166, 229
            ],
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_headers_hash(
                Some(&logger),
                canonicalization_type,
                &headers,
                hash_algo,
                &dkim_header(),
                &email
            )
            .unwrap(),
            &[
                76, 143, 13, 248, 17, 209, 243, 111, 40, 96, 160, 242, 116, 86, 37, 249, 134, 253,
                196, 89, 6, 24, 157, 130, 142, 198, 27, 166, 127, 179, 72, 247
            ]
        )
    }

    #[test]
    fn test_compute_headers_hash_relaxed() {
        let email = mailparse::parse_mail(
            r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
            .as_bytes(),
        )
        .unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let hash_algo = HashAlgo::RsaSha1;
        let headers = "To: Subject".to_owned();
        let logger = slog::Logger::root(slog::Discard, slog::o!());
        assert_eq!(
            compute_headers_hash(
                Some(&logger),
                canonicalization_type.clone(),
                &headers,
                hash_algo,
                &dkim_header(),
                &email
            )
            .unwrap(),
            &[
                14, 171, 230, 1, 77, 117, 47, 207, 243, 167, 179, 5, 150, 82, 154, 25, 125, 124,
                44, 164
            ]
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_headers_hash(
                Some(&logger),
                canonicalization_type,
                &headers,
                hash_algo,
                &dkim_header(),
                &email
            )
            .unwrap(),
            &[
                45, 186, 211, 81, 49, 111, 18, 147, 180, 245, 207, 39, 9, 9, 118, 137, 248, 204,
                70, 214, 16, 98, 216, 111, 230, 130, 196, 3, 60, 201, 166, 224
            ]
        )
    }

    #[test]
    fn test_get_body() {
        let email =
            mailparse::parse_mail("Subject: A\r\n\r\nContent\n.hi\n.hello..".as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&get_body(&email).unwrap()),
            "Content\n.hi\n.hello..".to_owned()
        );
    }

    #[test]
    fn test_select_headers() {
        let dkim_headers1 = ["from", "subject", "to", "from"].join(":");
        let email1 = mailparse::parse_mail(
            b"from: biz\r\nfoo: bar\r\nfrom: baz\r\nsubject: boring\r\n\r\ntest",
        )
        .unwrap();

        let result1 = select_headers(&dkim_headers1, &email1).unwrap();
        assert_eq!(
            result1,
            vec![
                ("from".to_owned(), &b"baz"[..]),
                ("subject".to_owned(), &b"boring"[..]),
                ("from".to_owned(), &b"biz"[..]),
            ]
        );

        let dkim_headers2 = ["from", "subject", "to", "from"].join(":");
        let email2 =
            mailparse::parse_mail(b"From: biz\r\nFoo: bar\r\nSubject: Boring\r\n\r\ntest").unwrap();

        let result2 = select_headers(&dkim_headers2, &email2).unwrap();
        assert_eq!(
            result2,
            vec![
                ("From".to_owned(), &b"biz"[..]),
                ("Subject".to_owned(), &b"Boring"[..]),
            ]
        );
    }
}
