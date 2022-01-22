use indexmap::set::IndexSet;
use mailparse::MailHeaderMap;
use slog::debug;
use std::io::BufRead;
use std::io::BufReader;

use crate::canonicalization::{
    self, canonicalize_body_relaxed, canonicalize_body_simple, canonicalize_header_relaxed,
    canonicalize_header_simple,
};
use crate::{bytes, DKIMError, DKIMHeader};

#[derive(Debug, Clone)]
pub enum HashAlgo {
    RsaSha1,
    RsaSha256,
}

/// Get the body part of an email
/// De-transparency according to RFC 5321, Section 4.5.2
fn get_body<'a>(email: &'a mailparse::ParsedMail<'a>) -> Result<Vec<u8>, DKIMError> {
    let body = bytes::get_all_after(email.raw_bytes, b"\r\n\r\n");
    let mut reader = BufReader::new(body);

    let mut buffer = Vec::new();

    loop {
        let mut line = Vec::new();
        let byte_read = reader
            .read_until(b'\n', &mut line)
            .map_err(|_err| DKIMError::MalformedBody)?;
        if byte_read == 0 {
            break;
        }

        // Remove leading period
        if line[0] == b'.' {
            line.remove(0);
        }

        buffer.append(&mut line);
    }

    Ok(buffer)
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
    };
    Ok(base64::encode(&hash))
}

fn select_headers<'a, 'b>(
    headers: &'b str,
    email: &'a mailparse::ParsedMail<'a>,
) -> Result<Vec<(String, &'a [u8])>, DKIMError> {
    let mut signed_headers = vec![];

    // Transform the header list into a ordered set to deduplicate the headers
    // while precerving the order
    let headers: IndexSet<&str> = IndexSet::from_iter(headers.split(":"));

    for name in headers {
        let name = name.trim();
        if let Some(header) = email.headers.get_first_header(name) {
            signed_headers.push((name.to_owned(), header.get_value_raw()));
        }
    }

    Ok(signed_headers)
}

pub(crate) fn compute_headers_hash<'a, 'b>(
    logger: &slog::Logger,
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
            canonicalize_header_simple(&key, &value)
        } else {
            canonicalize_header_relaxed(&key, &value)
        };
        input.extend_from_slice(&canonicalized_value);
    }

    // Add the DKIM-Signature header in the hash. Remove the value of the
    // signature (b) first.
    {
        let sign = dkim_header.get_raw_tag("b").unwrap();
        let value = dkim_header.raw_bytes.replace(&sign, "");
        input.extend_from_slice(&"dkim-signature:".as_bytes());
        input.extend_from_slice(&value.as_bytes());
    }
    debug!(logger, "headers to hash: {:?}", input);

    let hash = match hash_algo {
        HashAlgo::RsaSha1 => hash_sha1(&input),
        HashAlgo::RsaSha256 => hash_sha256(&input),
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
            compute_body_hash(canonicalization_type, length.clone(), hash_algo, &email).unwrap(),
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
            compute_body_hash(canonicalization_type, length.clone(), hash_algo, &email).unwrap(),
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
            compute_body_hash(canonicalization_type, length.clone(), hash_algo, &email).unwrap(),
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
                &logger,
                canonicalization_type.clone(),
                &headers,
                hash_algo,
                &dkim_header(),
                &email
            )
            .unwrap(),
            &[
                139, 181, 80, 152, 144, 190, 55, 167, 172, 184, 152, 202, 222, 81, 169, 121, 20, 5,
                213, 151
            ],
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_headers_hash(
                &logger,
                canonicalization_type.clone(),
                &headers,
                hash_algo,
                &dkim_header(),
                &email
            )
            .unwrap(),
            &[
                34, 222, 85, 83, 216, 70, 124, 226, 60, 174, 156, 184, 140, 247, 178, 88, 76, 99,
                182, 251, 149, 224, 243, 172, 54, 202, 138, 72, 45, 45, 88, 9
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
                &logger,
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
                &logger,
                canonicalization_type.clone(),
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
            mailparse::parse_mail("Subject: A\r\n\r\nContent\n..hi\n..hello..".as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&get_body(&email).unwrap()),
            "Content\n.hi\n.hello..".to_owned()
        );
    }
}
