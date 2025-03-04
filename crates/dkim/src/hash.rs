use crate::header::HEADER;
use crate::{canonicalization, DKIMError, DKIMHeader, ParsedEmail};
use data_encoding::BASE64;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub enum HashAlgo {
    RsaSha1,
    RsaSha256,
    Ed25519Sha256,
}

impl HashAlgo {
    pub fn algo_name(&self) -> &'static str {
        match self {
            Self::RsaSha1 => "rsa-sha1",
            Self::RsaSha256 => "rsa-sha256",
            Self::Ed25519Sha256 => "ed25519-sha256",
        }
    }
}

pub(crate) struct LimitHasher {
    pub limit: usize,
    pub hashed: usize,
    pub hasher: HashImpl,
}

impl LimitHasher {
    pub fn hash(&mut self, bytes: &[u8]) {
        let remain = self.limit - self.hashed;
        let len = bytes.len().min(remain);
        self.hasher.hash(&bytes[..len]);
        self.hashed += len;
    }

    pub fn finalize(self) -> String {
        self.hasher.finalize()
    }

    #[cfg(test)]
    pub fn finalize_bytes(self) -> Vec<u8> {
        self.hasher.finalize_bytes()
    }
}

pub(crate) enum HashImpl {
    Sha1(Sha1),
    Sha256(Sha256),
    #[cfg(test)]
    Copy(Vec<u8>),
}

impl HashImpl {
    pub fn from_algo(algo: HashAlgo) -> Self {
        match algo {
            HashAlgo::RsaSha1 => Self::Sha1(Sha1::new()),
            HashAlgo::RsaSha256 | HashAlgo::Ed25519Sha256 => Self::Sha256(Sha256::new()),
        }
    }

    #[cfg(test)]
    pub fn copy_data() -> Self {
        Self::Copy(vec![])
    }

    pub fn hash(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha1(hasher) => hasher.update(bytes),
            Self::Sha256(hasher) => hasher.update(bytes),
            #[cfg(test)]
            Self::Copy(data) => data.extend_from_slice(bytes),
        }
    }

    pub fn finalize(self) -> String {
        match self {
            Self::Sha1(hasher) => BASE64.encode(&hasher.finalize()),
            Self::Sha256(hasher) => BASE64.encode(&hasher.finalize()),
            #[cfg(test)]
            Self::Copy(data) => String::from_utf8_lossy(&data).into(),
        }
    }

    pub fn finalize_bytes(self) -> Vec<u8> {
        match self {
            Self::Sha1(hasher) => hasher.finalize().to_vec(),
            Self::Sha256(hasher) => hasher.finalize().to_vec(),
            #[cfg(test)]
            Self::Copy(data) => data,
        }
    }
}

/// Returns the hash of message's body
/// https://datatracker.ietf.org/doc/html/rfc6376#section-3.7
pub(crate) fn compute_body_hash<'a>(
    canonicalization_type: canonicalization::Type,
    length: Option<usize>,
    hash_algo: HashAlgo,
    email: &'a ParsedEmail<'a>,
) -> Result<String, DKIMError> {
    let body = email.get_body();
    let limit = length.unwrap_or(usize::MAX);

    let mut hasher = LimitHasher {
        hasher: HashImpl::from_algo(hash_algo),
        limit,
        hashed: 0,
    };

    canonicalization_type.canon_body(body.as_bytes(), &mut hasher);

    Ok(hasher.finalize())
}

/// Holds a list of header names, normalized to lower case
#[derive(Debug)]
pub(crate) enum HeaderList {
    /// A list of possibly duplicated header names
    MaybeMultiple(Vec<String>),
    /// A list of of unique header names
    Unique(Vec<String>),
}

impl HeaderList {
    pub fn as_h_list(&self) -> String {
        match self {
            Self::MaybeMultiple(list) | Self::Unique(list) => list.join(":"),
        }
    }

    /// Computes the header list that should be used to over-sign
    /// the provided email message, and returns it
    pub fn compute_over_signed(&self, email: &ParsedEmail) -> Self {
        let unique_header_names = match self {
            Self::Unique(names) => names.clone(),
            Self::MaybeMultiple(names) => {
                // Note that Self::new() normalized the names, so we can
                // simply dedup without further concern for normalization
                let mut n = names.clone();
                n.sort();
                n.dedup();
                n
            }
        };

        let email_headers = email.get_headers();

        let mut result = vec![];
        for name in unique_header_names {
            for _ in email_headers.iter_named(&name) {
                result.push(name.clone());
            }
            result.push(name);
        }

        Self::MaybeMultiple(result)
    }

    /// Build a header list.
    /// Analyzes the list to determine whether it is a unique list or not
    pub fn new(list: Vec<String>) -> Self {
        let normalized: Vec<String> = list.into_iter().map(|s| s.to_ascii_lowercase()).collect();

        let mut all_single = true;
        for name in &normalized {
            let n: usize = normalized
                .iter()
                .map(|candidate| if candidate == name { 1 } else { 0 })
                .sum();
            if n > 1 {
                all_single = false;
                break;
            }
        }

        if all_single {
            Self::Unique(normalized)
        } else {
            Self::MaybeMultiple(normalized)
        }
    }

    /// Apply `apply` to each header in the provided email that
    /// matches the headers, follow the order set out in Section 5.4.2
    fn apply<'a, F: FnMut(&'a str, &'a [u8])>(&self, email: &'a ParsedEmail, apply: F) {
        match self {
            Self::MaybeMultiple(list) => Self::apply_multiple(list, email, apply),
            Self::Unique(list) => Self::apply_unique(list, email, apply),
        }
    }

    /// Perform the apply when we know that the list of header names
    /// are unique.
    /// We can avoid allocating any additional state for this case.
    fn apply_unique<'a, F: FnMut(&'a str, &'a [u8])>(
        header_list: &[String],
        email: &'a ParsedEmail,
        mut apply: F,
    ) {
        let email_headers = email.get_headers();

        'outer: for name in header_list {
            for header in email_headers.iter().rev() {
                if header.get_name().eq_ignore_ascii_case(&name) {
                    apply(header.get_name(), header.get_raw_value().as_bytes());
                    continue 'outer;
                }
            }
        }
    }

    /// Section 5.4.2:
    /// Signers wishing to sign multiple instances of such a header field MUST
    /// include the header field name multiple times in the "h=" tag of the
    /// DKIM-Signature header field and MUST sign such header fields in order
    /// from the bottom of the header field block to the top.
    ///
    /// To facilitate this, we need to maintain state for each header name
    /// in the list to ensure that we select the appropriate header in the
    /// appropriate order.
    fn apply_multiple<'a, F: FnMut(&'a str, &'a [u8])>(
        header_list: &[String],
        email: &'a ParsedEmail,
        mut apply: F,
    ) {
        let email_headers = email.get_headers();
        let num_headers = email_headers.len();

        // Note: this map only works correctly if the header names are normalized
        // to lower case.  That happens in our constructor.
        let mut last_index: HashMap<&String, usize> = HashMap::new();

        'outer: for name in header_list {
            let index = last_index.get(name).unwrap_or(&num_headers);
            for (header_index, header) in email_headers
                .iter()
                .enumerate()
                .rev()
                .skip(num_headers - index)
            {
                if header.get_name().eq_ignore_ascii_case(&name) {
                    apply(header.get_name(), header.get_raw_value().as_bytes());
                    last_index.insert(name, header_index);
                    continue 'outer;
                }
            }

            // When computing the signature, the nonexisting header field MUST be
            // treated as the null string (including the header field name, header
            // field value, all punctuation, and the trailing CRLF).
            // -> don't include it in the returned signed_headers.

            last_index.insert(name, 0);
        }
    }
}

pub(crate) fn compute_headers_hash<'a, 'b>(
    canonicalization_type: canonicalization::Type,
    headers: &HeaderList,
    hash_algo: HashAlgo,
    dkim_header: &'b DKIMHeader,
    email: &'a ParsedEmail<'a>,
) -> Result<Vec<u8>, DKIMError> {
    let mut input = Vec::new();
    let mut hasher = HashImpl::from_algo(hash_algo);

    headers.apply(email, |key, value| {
        canonicalization_type.canon_header_into(&key, value, &mut input);
    });

    // Add the DKIM-Signature header in the hash. Remove the value of the
    // signature (b) first.
    {
        let sign = dkim_header.get_required_raw_tag("b");
        let value = dkim_header.raw_bytes.replace(&sign, "");
        let mut canonicalized_value = vec![];
        canonicalization_type.canon_header_into(HEADER, value.as_bytes(), &mut canonicalized_value);

        // remove trailing "\r\n"
        canonicalized_value.truncate(canonicalized_value.len() - 2);

        input.extend_from_slice(&canonicalized_value);
    }
    tracing::debug!("headers to hash: {:?}", input);

    hasher.hash(&input);
    let hash = hasher.finalize_bytes();
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dkim_header() -> DKIMHeader {
        crate::DKIMHeader::parse("v=1; a=rsa-sha256; q=dns/txt; c=relaxed/relaxed; s=smtp; d=test.com; t=1641506955; h=content-type:to: subject:date:from:mime-version:sender; bh=PU2XIErWsXvhvt1W96ntPWZ2VImjVZ3vBY2T/A+wA3A=; b=PIO0A014nyntOGKdTdtvCJor9ZxvP1M3hoLeEh8HqZ+RvAyEKdAc7VOg+/g/OTaZgsmw6U sZCoN0YNVp+2o9nkaeUslsVz3M4I55HcZnarxl+fhplIMcJ/3s0nIhXL51MfGPRqPbB7/M Gjg9/07/2vFoid6Kitg6Z+CfoD2wlSRa8xDfmeyA2cHpeVuGQhGxu7BXuU8kGbeM4+weit Ql3t9zalhikEPI5Pr7dzYFrgWNOEO6w6rQfG7niKON1BimjdbJlGanC7cO4UL361hhXT4X iXLnC9TG39xKFPT/+4nkHy8pp6YvWkD3wKlBjwkYNm0JvKGwTskCMDeTwxXhAg==").unwrap()
    }

    #[test]
    fn test_compute_body_hash_simple() {
        let email = r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(email).unwrap();

        let canonicalization_type = canonicalization::Type::Simple;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "ya82MJvChLGBNSxeRvrSat5LliQ="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "KXQwQpX2zFwgixPbV6Dd18ZMJU04lLeRnwqzUp8uGwI=",
        )
    }

    #[test]
    fn test_compute_body_hash_relaxed() {
        let email = r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(email).unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "wpj48VhihzV7I31ZZZUp1UpTyyM="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "1bokzbYiRgXTKMQhrNhLJo1kjDDA1GILbpyTwyNa1uk=",
        )
    }

    #[test]
    fn test_compute_body_hash_length() {
        let email = r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(email).unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let length = Some(3);
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "28LR/tDcN6cK6g83aVjIAu3cBVk="
        );
        let hash_algo = HashAlgo::RsaSha256;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
            "t4nCTc22jEQ3sEwYa/I5pyB+dXP7GyKnSf4ae42W0pI=",
        )
    }

    #[test]
    fn test_compute_body_hash_empty_simple() {
        let email = ParsedEmail::parse("Subject: nothing\r\n\r\n").unwrap();

        let canonicalization_type = canonicalization::Type::Simple;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
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
        let email = ParsedEmail::parse("Subject: nothing\r\n\r\n").unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let length = None;
        let hash_algo = HashAlgo::RsaSha1;
        assert_eq!(
            compute_body_hash(canonicalization_type, length, hash_algo, &email).unwrap(),
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
        let email = r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(email).unwrap();

        let canonicalization_type = canonicalization::Type::Simple;
        let hash_algo = HashAlgo::RsaSha1;
        let headers = HeaderList::new(vec!["To".to_owned(), "Subject".to_owned()]);
        assert_eq!(
            compute_headers_hash(
                canonicalization_type,
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
        let email = r#"To: test@sauleau.com
Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
        "#
        .replace("\n", "\r\n");
        let email = ParsedEmail::parse(email).unwrap();

        let canonicalization_type = canonicalization::Type::Relaxed;
        let hash_algo = HashAlgo::RsaSha1;
        let headers = HeaderList::new(vec!["To".to_owned(), "Subject".to_owned()]);
        assert_eq!(
            compute_headers_hash(
                canonicalization_type,
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
        let email = ParsedEmail::parse("Subject: A\r\n\r\nContent\n.hi\n.hello..").unwrap();
        assert_eq!(email.get_body(), "Content\n.hi\n.hello..");
    }

    fn select_headers<'a>(
        header_list: &HeaderList,
        email: &'a ParsedEmail,
    ) -> Vec<(&'a str, &'a [u8])> {
        let mut result = vec![];
        header_list.apply(email, |key, value| {
            result.push((key, value));
        });
        result
    }

    #[test]
    fn test_select_headers_unique() {
        let header_list = HeaderList::new(vec![
            "from".to_string(),
            "subject".to_string(),
            "to".to_string(),
        ]);

        let email1 =
            ParsedEmail::parse("from: biz\r\nfoo: bar\r\nfrom: baz\r\nsubject: boring\r\n\r\ntest")
                .unwrap();

        let result1 = select_headers(&header_list, &email1);
        assert_eq!(
            result1,
            vec![
                ("from".into(), &b"baz"[..]),
                ("subject".into(), &b"boring"[..]),
            ]
        );

        let email2 =
            ParsedEmail::parse("From: biz\r\nFoo: bar\r\nSubject: Boring\r\n\r\ntest").unwrap();

        let result2 = select_headers(&header_list, &email2);
        assert_eq!(
            result2,
            vec![
                ("From".into(), &b"biz"[..]),
                ("Subject".into(), &b"Boring"[..]),
            ]
        );
    }

    #[test]
    fn test_select_headers_multiple() {
        let header_list = HeaderList::new(vec![
            "from".to_string(),
            "subject".to_string(),
            "to".to_string(),
            "from".to_string(),
        ]);

        let email1 =
            ParsedEmail::parse("from: biz\r\nfoo: bar\r\nfrom: baz\r\nsubject: boring\r\n\r\ntest")
                .unwrap();

        let result1 = select_headers(&header_list, &email1);
        assert_eq!(
            result1,
            vec![
                ("from".into(), &b"baz"[..]),
                ("subject".into(), &b"boring"[..]),
                ("from".into(), &b"biz"[..]),
            ]
        );

        let email2 =
            ParsedEmail::parse("From: biz\r\nFoo: bar\r\nSubject: Boring\r\n\r\ntest").unwrap();

        let result2 = select_headers(&header_list, &email2);
        assert_eq!(
            result2,
            vec![
                ("From".into(), &b"biz"[..]),
                ("Subject".into(), &b"Boring"[..]),
            ]
        );
    }
}
