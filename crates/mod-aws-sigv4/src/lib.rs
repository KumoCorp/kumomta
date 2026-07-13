use anyhow::Context;
use aws_lc_rs::hmac::Key;
use chrono::{DateTime, Utc};
use config::{any_err, from_lua_value, get_or_create_sub_module};
use data_encoding::HEXLOWER;
use data_loader::KeySource;
use mlua::{Lua, LuaSerdeExt, Value};
use percent_encoding::{percent_encode, AsciiSet, CONTROLS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// AWS SigV4 URI encoding set
/// Encodes everything except: A-Z a-z 0-9 - _ . ~
const URI_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b']');

#[derive(Deserialize, Debug)]
pub struct SigV4Request {
    /// AWS access key ID (can be a KeySource)
    pub access_key: KeySource,
    /// AWS secret access key (can be a KeySource)
    pub secret_key: KeySource,
    /// AWS region (e.g., "us-east-1")
    pub region: String,
    /// AWS service name (e.g., "s3", "sns", "sqs")
    pub service: String,
    /// HTTP method (e.g., "GET", "POST")
    pub method: String,
    /// URI path (e.g., "/")
    pub uri: String,
    /// Optional query string parameters
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    /// HTTP headers to sign
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Request payload (body)
    #[serde(default)]
    pub payload: String,
    /// Optional timestamp (defaults to current time)
    pub timestamp: Option<DateTime<Utc>>,
    /// Optional session token for temporary credentials
    pub session_token: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SigV4Response {
    /// The authorization header value
    pub authorization: String,
    /// The timestamp used in ISO8601 format (YYYYMMDD'T'HHMMSS'Z')
    pub timestamp: String,
    /// The canonical request (for debugging)
    pub canonical_request: String,
    /// The string to sign (for debugging)
    pub string_to_sign: String,
    /// The signature
    pub signature: String,
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let key = Key::new(aws_lc_rs::hmac::HMAC_SHA256, key);
    let tag = aws_lc_rs::hmac::sign(&key, data);
    tag.as_ref().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    use aws_lc_rs::digest;
    let hash = digest::digest(&digest::SHA256, data);
    HEXLOWER.encode(hash.as_ref())
}

fn uri_encode(input: &str) -> String {
    percent_encode(input.as_bytes(), URI_ENCODE_SET).to_string()
}

fn create_canonical_uri(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        // Split path and encode each segment
        path.split('/')
            .map(uri_encode)
            .collect::<Vec<_>>()
            .join("/")
    }
}

fn create_canonical_query_string(params: &BTreeMap<String, String>) -> String {
    if params.is_empty() {
        return String::new();
    }

    // Sort parameters and URI encode them.
    //
    // We collect into a Vec and sort on the *encoded* keys to ensure
    // the ordering is correct even when encoding changes the byte
    // ordering of the original key/value strings.
    let mut encoded_params: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (uri_encode(k), uri_encode(v)))
        .collect();
    encoded_params.sort();

    encoded_params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

/// AWS SigV4 Trimall: per the spec, remove leading/trailing ASCII SP/HTAB
/// and collapse internal runs of SP/HTAB to a single space, but do NOT
/// touch whitespace inside double-quoted string segments.
///
/// See: <https://docs.aws.amazon.com/IAM/latest/UserGuide/create-signed-request.html>
/// ("Trimall function").
fn trimall(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut in_quotes = false;
    let mut pending_space = false;
    let mut have_emitted = false;

    for &b in bytes {
        if b == b'"' {
            if pending_space && have_emitted {
                out.push(' ');
            }
            pending_space = false;
            out.push('"');
            have_emitted = true;
            in_quotes = !in_quotes;
            continue;
        }

        if in_quotes {
            out.push(b as char);
            continue;
        }

        if b == b' ' || b == b'\t' {
            pending_space = true;
            continue;
        }

        if pending_space && have_emitted {
            out.push(' ');
        }
        pending_space = false;
        out.push(b as char);
        have_emitted = true;
    }

    out
}

fn create_canonical_headers(headers: &BTreeMap<String, String>) -> (String, String) {
    // Convert header names to lowercase and apply AWS Trimall to values.
    let canonical_headers: BTreeMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), trimall(v)))
        .collect();

    // Sort headers
    let header_string = canonical_headers
        .iter()
        .map(|(k, v)| format!("{}:{}", k, v))
        .collect::<Vec<_>>()
        .join("\n");

    // Create signed headers list
    let signed_headers = canonical_headers
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(";");

    (header_string, signed_headers)
}

fn create_signing_key(secret_key: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(
        format!("AWS4{secret_key}").as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

pub async fn sign_request(req: SigV4Request) -> anyhow::Result<SigV4Response> {
    // Get the access key id and secret key from their KeySource values
    let access_key_bytes = req.access_key.get().await?;
    let access_key = std::str::from_utf8(&access_key_bytes)
        .context("access_key must be valid UTF-8")?
        .to_string();

    // Get the secret key
    let secret_key_bytes = req.secret_key.get().await?;
    let secret_key = std::str::from_utf8(&secret_key_bytes)
        .context("secret_key must be valid UTF-8")?
        .to_string();

    // Use provided timestamp or current time
    let timestamp = req.timestamp.unwrap_or_else(Utc::now);
    let amz_date = timestamp.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = timestamp.format("%Y%m%d").to_string();

    // Create payload hash
    let payload_hash = sha256_hex(req.payload.as_bytes());

    // Prepare headers - add required AWS headers. Normalize all caller-supplied
    // header names to lowercase up front so downstream checks (e.g. the 'host'
    // precondition) and de-duplication work regardless of input casing.
    let mut headers: BTreeMap<String, String> = req
        .headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();

    anyhow::ensure!(headers.contains_key("host"), "headers must include 'host'");

    headers.insert("x-amz-date".to_string(), amz_date.clone());

    if let Some(token) = &req.session_token {
        headers.insert("x-amz-security-token".to_string(), token.clone());
    }

    // S3 requires x-amz-content-sha256 to be signed; other services do not.
    // Callers can include it for non-S3 services via req.headers if needed.
    if req.service == "s3" {
        headers.insert("x-amz-content-sha256".to_string(), payload_hash.clone());
    }

    // Create canonical request
    let canonical_uri = create_canonical_uri(&req.uri);
    let canonical_query_string = create_canonical_query_string(&req.query_params);
    let (canonical_headers, signed_headers) = create_canonical_headers(&headers);

    // See https://docs.aws.amazon.com/general/latest/gr/sigv4-create-canonical-request.html
    // for the canonical request structure. The blank line between the
    // canonical headers and the signed headers is required by the spec.
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query_string}\n{canonical_headers}\n\n{signed_headers}\n{payload_hash}",
        method = req.method,
    );

    // Create string to sign
    let algorithm = "AWS4-HMAC-SHA256";
    let credential_scope = format!(
        "{date_stamp}/{region}/{service}/aws4_request",
        region = req.region,
        service = req.service
    );
    let canonical_request_hash = sha256_hex(canonical_request.as_bytes());

    let string_to_sign =
        format!("{algorithm}\n{amz_date}\n{credential_scope}\n{canonical_request_hash}");

    // Calculate signature
    let signing_key = create_signing_key(&secret_key, &date_stamp, &req.region, &req.service);
    let signature_bytes = hmac_sha256(&signing_key, string_to_sign.as_bytes());
    let signature = HEXLOWER.encode(&signature_bytes);

    // Create authorization header
    let authorization = format!(
        "{algorithm} Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        access_key = access_key
    );

    Ok(SigV4Response {
        authorization,
        timestamp: amz_date,
        canonical_request,
        string_to_sign,
        signature,
    })
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    // Register under kumo.crypto as aws_sign_v4 so that the function
    // shows up alongside the other crypto helpers in the reference docs.
    let aws_mod = get_or_create_sub_module(lua, "crypto")?;

    aws_mod.set(
        "aws_sign_v4",
        lua.create_async_function(|lua, request: Value| async move {
            let req: SigV4Request = from_lua_value(&lua, request)?;
            let response = sign_request(req).await.map_err(any_err)?;

            lua.to_value(&response)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_encode() {
        assert_eq!(uri_encode("test"), "test");
        assert_eq!(uri_encode("test value"), "test%20value");
        assert_eq!(uri_encode("test/path"), "test%2Fpath");
        assert_eq!(uri_encode("test-value_123.txt~"), "test-value_123.txt~");
    }

    #[test]
    fn test_canonical_uri() {
        assert_eq!(create_canonical_uri(""), "/");
        assert_eq!(create_canonical_uri("/"), "/");
        assert_eq!(create_canonical_uri("/path"), "/path");
        assert_eq!(create_canonical_uri("/path/to/file"), "/path/to/file");
        assert_eq!(
            create_canonical_uri("/path with spaces"),
            "/path%20with%20spaces"
        );
    }

    #[test]
    fn test_canonical_query_string() {
        let mut params = BTreeMap::new();
        assert_eq!(create_canonical_query_string(&params), "");

        params.insert("key".to_string(), "value".to_string());
        assert_eq!(create_canonical_query_string(&params), "key=value");

        params.insert("another".to_string(), "test".to_string());
        assert_eq!(
            create_canonical_query_string(&params),
            "another=test&key=value"
        );
    }

    #[test]
    fn test_sha256_hex() {
        let result = sha256_hex(b"test");
        assert_eq!(
            result,
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }

    #[test]
    fn test_hmac_sha256() {
        let result = hmac_sha256(b"key", b"message");
        let hex = HEXLOWER.encode(&result);
        assert_eq!(
            hex,
            "6e9ef29b75fffc5b7abae527d58fdadb2fe42e7219011976917343065f58ed4a"
        );
    }

    #[test]
    fn test_signing_key_derivation() {
        // Test vector based on AWS documentation
        let signing_key = create_signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830",
            "us-east-1",
            "iam",
        );
        let hex = HEXLOWER.encode(&signing_key);
        assert_eq!(
            hex,
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    #[tokio::test]
    async fn test_sign_request_basic() {
        // Test the full sign_request function with inline key data
        let req = SigV4Request {
            access_key: KeySource::Data {
                key_data: b"AKIAIOSFODNN7EXAMPLE".to_vec(),
            },
            secret_key: KeySource::Data {
                key_data: b"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_vec(),
            },
            region: "us-east-1".to_string(),
            service: "iam".to_string(),
            method: "GET".to_string(),
            uri: "/".to_string(),
            query_params: BTreeMap::new(),
            headers: {
                let mut h = BTreeMap::new();
                h.insert("host".to_string(), "iam.amazonaws.com".to_string());
                h
            },
            payload: String::new(),
            timestamp: Some(
                DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            session_token: None,
        };

        let response = sign_request(req).await.expect("signing should succeed");

        // Verify the response contains expected components
        assert!(response.authorization.starts_with("AWS4-HMAC-SHA256"));
        assert!(response
            .authorization
            .contains("Credential=AKIAIOSFODNN7EXAMPLE/20150830/us-east-1/iam/aws4_request"));
        assert_eq!(response.timestamp, "20150830T123600Z");
        // Signature should be a 64-character hex string
        assert_eq!(response.signature.len(), 64);
        assert!(response.signature.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn test_sign_request_with_query_params() {
        let mut query_params = BTreeMap::new();
        query_params.insert("Action".to_string(), "ListUsers".to_string());
        query_params.insert("Version".to_string(), "2010-05-08".to_string());

        let req = SigV4Request {
            access_key: KeySource::Data {
                key_data: b"AKIAIOSFODNN7EXAMPLE".to_vec(),
            },
            secret_key: KeySource::Data {
                key_data: b"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_vec(),
            },
            region: "us-east-1".to_string(),
            service: "iam".to_string(),
            method: "GET".to_string(),
            uri: "/".to_string(),
            query_params,
            headers: {
                let mut h = BTreeMap::new();
                h.insert("host".to_string(), "iam.amazonaws.com".to_string());
                h
            },
            payload: String::new(),
            timestamp: Some(
                DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            session_token: None,
        };

        let response = sign_request(req).await.expect("signing should succeed");

        // Verify query params are included in the canonical request
        assert!(response.canonical_request.contains("Action=ListUsers"));
        assert!(response.canonical_request.contains("Version=2010-05-08"));
    }

    // Credentials shared across all official AWS test vectors.
    fn official_vector_access_key() -> KeySource {
        KeySource::Data {
            key_data: b"AKIDEXAMPLE".to_vec(),
        }
    }
    fn official_vector_secret_key() -> KeySource {
        KeySource::Data {
            key_data: b"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_vec(),
        }
    }
    fn official_vector_timestamp() -> Option<DateTime<Utc>> {
        Some(
            DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }
    fn official_vector_headers() -> BTreeMap<String, String> {
        let mut h = BTreeMap::new();
        h.insert("host".to_string(), "example.amazonaws.com".to_string());
        h
    }

    // Test vectors from the official AWS SigV4 test suite:
    // https://docs.aws.amazon.com/general/latest/gr/sigv4-test-suite.html
    // (mirrored at https://github.com/saibotsivad/aws-sig-v4-test-suite)
    #[tokio::test]
    async fn test_official_vector_get_vanilla() {
        let req = SigV4Request {
            access_key: official_vector_access_key(),
            secret_key: official_vector_secret_key(),
            region: "us-east-1".to_string(),
            service: "service".to_string(),
            method: "GET".to_string(),
            uri: "/".to_string(),
            query_params: BTreeMap::new(),
            headers: official_vector_headers(),
            payload: String::new(),
            timestamp: official_vector_timestamp(),
            session_token: None,
        };

        let response = sign_request(req).await.expect("signing should succeed");

        k9::assert_equal!(
            response.canonical_request,
            "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        k9::assert_equal!(
            response.string_to_sign,
            "AWS4-HMAC-SHA256\n20150830T123600Z\n20150830/us-east-1/service/aws4_request\nbb579772317eb040ac9ed261061d46c1f17a8133879d6129b6e1c25292927e63"
        );
        k9::assert_equal!(
            response.signature,
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        k9::assert_equal!(
            response.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[tokio::test]
    async fn test_official_vector_post_vanilla() {
        let req = SigV4Request {
            access_key: official_vector_access_key(),
            secret_key: official_vector_secret_key(),
            region: "us-east-1".to_string(),
            service: "service".to_string(),
            method: "POST".to_string(),
            uri: "/".to_string(),
            query_params: BTreeMap::new(),
            headers: official_vector_headers(),
            payload: String::new(),
            timestamp: official_vector_timestamp(),
            session_token: None,
        };

        let response = sign_request(req).await.expect("signing should succeed");

        k9::assert_equal!(
            response.signature,
            "5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"
        );
        k9::assert_equal!(
            response.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"
        );
    }

    #[test]
    fn test_trimall() {
        // Basic trim + collapse.
        k9::assert_equal!(trimall("  foo  "), "foo");
        k9::assert_equal!(trimall("foo   bar  baz"), "foo bar baz");
        // Tabs treated like spaces.
        k9::assert_equal!(trimall("\tfoo\t\tbar\t"), "foo bar");
        // Whitespace inside quoted strings is preserved verbatim.
        k9::assert_equal!(
            trimall("  a   \"keep  these   spaces\"   b  "),
            "a \"keep  these   spaces\" b"
        );
        // Empty and whitespace-only inputs.
        k9::assert_equal!(trimall(""), "");
        k9::assert_equal!(trimall("   "), "");
    }

    #[test]
    fn test_canonical_headers_trimall() {
        let mut headers = BTreeMap::new();
        headers.insert("host".to_string(), "  example.com  ".to_string());
        headers.insert("x-custom".to_string(), "foo   bar  baz".to_string());
        let (header_string, _) = create_canonical_headers(&headers);
        assert!(
            header_string.contains("host:example.com"),
            "leading/trailing whitespace should be stripped: {header_string}"
        );
        assert!(
            header_string.contains("x-custom:foo bar baz"),
            "internal whitespace runs should collapse to one space: {header_string}"
        );
    }

    #[tokio::test]
    async fn test_mixed_case_host_accepted() {
        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), "example.amazonaws.com".to_string());
        let req = SigV4Request {
            access_key: official_vector_access_key(),
            secret_key: official_vector_secret_key(),
            region: "us-east-1".to_string(),
            service: "service".to_string(),
            method: "GET".to_string(),
            uri: "/".to_string(),
            query_params: BTreeMap::new(),
            headers,
            payload: String::new(),
            timestamp: official_vector_timestamp(),
            session_token: None,
        };
        // Should match the lowercase-host vector exactly.
        let response = sign_request(req).await.expect("signing should succeed");
        k9::assert_equal!(
            response.signature,
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[tokio::test]
    async fn test_s3_includes_content_sha256() {
        let req = SigV4Request {
            access_key: KeySource::Data {
                key_data: b"AKIAIOSFODNN7EXAMPLE".to_vec(),
            },
            secret_key: KeySource::Data {
                key_data: b"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_vec(),
            },
            region: "us-east-1".to_string(),
            service: "s3".to_string(),
            method: "GET".to_string(),
            uri: "/my-bucket/my-key".to_string(),
            query_params: BTreeMap::new(),
            headers: {
                let mut h = BTreeMap::new();
                h.insert("host".to_string(), "s3.amazonaws.com".to_string());
                h
            },
            payload: String::new(),
            timestamp: Some(
                DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            session_token: None,
        };

        let response = sign_request(req).await.expect("signing should succeed");

        assert!(
            response.canonical_request.contains("x-amz-content-sha256"),
            "S3 requests must include x-amz-content-sha256 in the canonical request"
        );
        assert!(
            response.authorization.contains("x-amz-content-sha256"),
            "S3 requests must sign x-amz-content-sha256"
        );
    }

    #[tokio::test]
    async fn test_missing_host_returns_error() {
        let req = SigV4Request {
            access_key: KeySource::Data {
                key_data: b"AKIAIOSFODNN7EXAMPLE".to_vec(),
            },
            secret_key: KeySource::Data {
                key_data: b"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_vec(),
            },
            region: "us-east-1".to_string(),
            service: "iam".to_string(),
            method: "GET".to_string(),
            uri: "/".to_string(),
            query_params: BTreeMap::new(),
            headers: BTreeMap::new(), // no host
            payload: String::new(),
            timestamp: None,
            session_token: None,
        };

        let err = sign_request(req).await.unwrap_err().to_string();
        k9::assert_equal!(err, "headers must include 'host'");
    }

    #[tokio::test]
    async fn test_sign_request_with_session_token() {
        let req = SigV4Request {
            access_key: KeySource::Data {
                key_data: b"AKIAIOSFODNN7EXAMPLE".to_vec(),
            },
            secret_key: KeySource::Data {
                key_data: b"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_vec(),
            },
            region: "us-east-1".to_string(),
            service: "sts".to_string(),
            method: "GET".to_string(),
            uri: "/".to_string(),
            query_params: BTreeMap::new(),
            headers: {
                let mut h = BTreeMap::new();
                h.insert("host".to_string(), "sts.amazonaws.com".to_string());
                h
            },
            payload: String::new(),
            timestamp: Some(
                DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            session_token: Some("AQoDYXdzEJr...".to_string()),
        };

        let response = sign_request(req).await.expect("signing should succeed");

        // Verify session token header is included in signed headers
        assert!(response.canonical_request.contains("x-amz-security-token"));
    }
}
