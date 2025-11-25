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
struct SigV4Request {
    /// AWS access key ID (can be a KeySource)
    access_key: KeySource,
    /// AWS secret access key (can be a KeySource)
    secret_key: KeySource,
    /// AWS region (e.g., "us-east-1")
    region: String,
    /// AWS service name (e.g., "s3", "sns", "sqs")
    service: String,
    /// HTTP method (e.g., "GET", "POST")
    method: String,
    /// URI path (e.g., "/")
    uri: String,
    /// Optional query string parameters
    #[serde(default)]
    query_params: BTreeMap<String, String>,
    /// HTTP headers to sign
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Request payload (body)
    #[serde(default)]
    payload: String,
    /// Optional timestamp (defaults to current time)
    timestamp: Option<DateTime<Utc>>,
    /// Optional session token for temporary credentials
    session_token: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
struct SigV4Response {
    /// The authorization header value
    authorization: String,
    /// The timestamp used in ISO8601 format (YYYYMMDD'T'HHMMSS'Z')
    timestamp: String,
    /// The canonical request (for debugging)
    canonical_request: String,
    /// The string to sign (for debugging)
    string_to_sign: String,
    /// The signature
    signature: String,
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

fn create_canonical_headers(headers: &BTreeMap<String, String>) -> (String, String) {
    // Convert headers to lowercase and trim values
    let canonical_headers: BTreeMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_string()))
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

async fn sign_request(req: SigV4Request) -> anyhow::Result<SigV4Response> {
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

    // Prepare headers - add required AWS headers
    let mut headers = req.headers.clone();
    headers.insert("host".to_string(), "".to_string()); // Will be set by caller
    headers.insert("x-amz-date".to_string(), amz_date.clone());

    if let Some(token) = &req.session_token {
        headers.insert("x-amz-security-token".to_string(), token.clone());
    }

    // Add content hash header for some services
    if req.service != "s3" {
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
}
