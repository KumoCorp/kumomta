use kumo_server_common::authn_authz::IdentityContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaslMech {
    Plain,
    OAuthBearer,
    XOAuth2,
}

impl SaslMech {
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Plain, Self::OAuthBearer, Self::XOAuth2]
            .into_iter()
            .find(|mech| mech.name() == name)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Plain => "PLAIN",
            Self::OAuthBearer => "OAUTHBEARER",
            Self::XOAuth2 => "XOAUTH2",
        }
    }

    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Plain => "smtp_server_auth_plain",
            Self::OAuthBearer => "smtp_server_auth_oauthbearer",
            Self::XOAuth2 => "smtp_server_auth_xoauth2",
        }
    }

    pub fn authentication_context(&self) -> IdentityContext {
        match self {
            Self::Plain => IdentityContext::SmtpAuthPlainAuthentication,
            Self::OAuthBearer => IdentityContext::SmtpAuthOAuthBearerAuthentication,
            Self::XOAuth2 => IdentityContext::SmtpAuthXOAuth2Authentication,
        }
    }

    pub fn authorization_context(&self) -> IdentityContext {
        match self {
            Self::Plain => IdentityContext::SmtpAuthPlainAuthorization,
            Self::OAuthBearer => IdentityContext::SmtpAuthOAuthBearerAuthorization,
            Self::XOAuth2 => IdentityContext::SmtpAuthXOAuth2Authorization,
        }
    }

    /// The base64 encoded error challenge sent before the final failure
    /// response. The client answers it with a dummy response; see
    /// RFC 7628 section 3.2.3 and the Google XOAUTH2 protocol docs.
    pub fn failure_challenge(&self) -> Option<&'static str> {
        match self {
            Self::Plain => None,
            // {"status":"invalid_token"}
            Self::OAuthBearer => Some("eyJzdGF0dXMiOiJpbnZhbGlkX3Rva2VuIn0="),
            // {"status":"401","schemes":"bearer"}
            Self::XOAuth2 => Some("eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiYmVhcmVyIn0="),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct BearerResponse {
    /// The OAUTHBEARER authzid or the XOAUTH2 user; may be empty
    /// for OAUTHBEARER
    pub user: String,
    pub token: String,
}

/// Decodes an RFC 7628 OAUTHBEARER client response:
/// `gs2-header %x01 *(key=value %x01) %x01`
pub fn decode_oauthbearer(payload: &[u8]) -> Option<BearerResponse> {
    let payload = std::str::from_utf8(payload).ok()?;
    let (gs2_header, kvpairs) = payload.split_once('\x01')?;

    let mut header = gs2_header.split(',');
    if !matches!(header.next(), Some("n" | "y")) {
        return None;
    }
    let user = match header.next()? {
        "" => String::new(),
        authzid => decode_saslname(authzid.strip_prefix("a=")?)?,
    };
    if header.next() != Some("") || header.next().is_some() {
        return None;
    }

    let token = bearer_token(kvpairs)?;
    Some(BearerResponse { user, token })
}

/// Decodes an XOAUTH2 client response:
/// `user=<user> %x01 auth=Bearer <token> %x01 %x01`
pub fn decode_xoauth2(payload: &[u8]) -> Option<BearerResponse> {
    let payload = std::str::from_utf8(payload).ok()?;
    let user = kvpairs(payload)?.find_map(|(key, value)| (key == "user").then_some(value))?;
    let token = bearer_token(payload)?;
    Some(BearerResponse {
        user: user.to_string(),
        token,
    })
}

fn kvpairs(s: &str) -> Option<impl Iterator<Item = (&str, &str)>> {
    let pairs = s.strip_suffix("\x01\x01")?;
    Some(pairs.split('\x01').filter_map(|pair| pair.split_once('=')))
}

fn bearer_token(s: &str) -> Option<String> {
    let auth = kvpairs(s)?.find_map(|(key, value)| (key == "auth").then_some(value))?;
    let token = auth.strip_prefix("Bearer ")?;
    if token.is_empty() || token.contains(' ') {
        return None;
    }
    Some(token.to_string())
}

/// RFC 5801 saslname escapes `,` as `=2C` and `=` as `=3D`
fn decode_saslname(name: &str) -> Option<String> {
    let mut result = String::with_capacity(name.len());
    let mut iter = name.split('=');
    result.push_str(iter.next()?);
    for chunk in iter {
        if let Some(rest) = chunk.strip_prefix("2C") {
            result.push(',');
            result.push_str(rest);
        } else if let Some(rest) = chunk.strip_prefix("3D") {
            result.push('=');
            result.push_str(rest);
        } else {
            return None;
        }
    }
    Some(result)
}

#[cfg(test)]
mod test {
    use super::*;

    fn bearer(user: &str, token: &str) -> Option<BearerResponse> {
        Some(BearerResponse {
            user: user.to_string(),
            token: token.to_string(),
        })
    }

    #[test]
    fn parse_mech() {
        assert_eq!(SaslMech::parse("PLAIN"), Some(SaslMech::Plain));
        assert_eq!(SaslMech::parse("OAUTHBEARER"), Some(SaslMech::OAuthBearer));
        assert_eq!(SaslMech::parse("XOAUTH2"), Some(SaslMech::XOAuth2));
        assert_eq!(SaslMech::parse("LOGIN"), None);
    }

    #[test]
    fn oauthbearer() {
        // Example from RFC 7628 section 4.1
        assert_eq!(
            decode_oauthbearer(
                b"n,a=user@example.com,\x01host=server.example.com\x01port=143\x01\
                  auth=Bearer vF9dft4qmTc2Nvb3RlckBhbHRhdmlzdGEuY29tCg==\x01\x01"
            ),
            bearer(
                "user@example.com",
                "vF9dft4qmTc2Nvb3RlckBhbHRhdmlzdGEuY29tCg=="
            )
        );
        assert_eq!(
            decode_oauthbearer(b"n,,\x01auth=Bearer tok\x01\x01"),
            bearer("", "tok")
        );
        assert_eq!(
            decode_oauthbearer(b"y,a=a=3Db=2Cc,\x01auth=Bearer tok\x01\x01"),
            bearer("a=b,c", "tok")
        );
    }

    #[test]
    fn oauthbearer_invalid() {
        // the dummy response sent after a failure challenge
        assert_eq!(decode_oauthbearer(b"\x01"), None);
        // channel binding is not supported
        assert_eq!(
            decode_oauthbearer(b"p=tls-unique,,\x01auth=Bearer tok\x01\x01"),
            None
        );
        assert_eq!(decode_oauthbearer(b"n,,\x01host=x\x01\x01"), None);
        assert_eq!(
            decode_oauthbearer(b"n,,\x01auth=Basic dXNlcg==\x01\x01"),
            None
        );
        assert_eq!(decode_oauthbearer(b"n,,\x01auth=Bearer \x01\x01"), None);
        assert_eq!(decode_oauthbearer(b"n,,\x01auth=Bearer tok\x01"), None);
        assert_eq!(
            decode_oauthbearer(b"n,u=user,\x01auth=Bearer tok\x01\x01"),
            None
        );
        assert_eq!(
            decode_oauthbearer(b"n,a=a=4,\x01auth=Bearer tok\x01\x01"),
            None
        );
        assert_eq!(decode_oauthbearer(b"n,,\x01auth=Bearer \xff\x01\x01"), None);
    }

    #[test]
    fn xoauth2() {
        assert_eq!(
            decode_xoauth2(b"user=someuser@example.com\x01auth=Bearer ya29.vF9dft4qmTc2Nvb3RlckBhbHRhdmlzdGEuY29tCg\x01\x01"),
            bearer(
                "someuser@example.com",
                "ya29.vF9dft4qmTc2Nvb3RlckBhbHRhdmlzdGEuY29tCg"
            )
        );
    }

    #[test]
    fn xoauth2_invalid() {
        assert_eq!(decode_xoauth2(b""), None);
        assert_eq!(decode_xoauth2(b"auth=Bearer tok\x01\x01"), None);
        assert_eq!(decode_xoauth2(b"user=u\x01\x01"), None);
        assert_eq!(decode_xoauth2(b"user=u\x01auth=Bearer tok\x01"), None);
    }

    #[test]
    fn failure_challenges() {
        use data_encoding::BASE64;
        let decode = |mech: SaslMech| {
            BASE64
                .decode(mech.failure_challenge().unwrap().as_bytes())
                .unwrap()
        };
        assert_eq!(
            decode(SaslMech::OAuthBearer),
            br#"{"status":"invalid_token"}"#
        );
        assert_eq!(
            decode(SaslMech::XOAuth2),
            br#"{"status":"401","schemes":"bearer"}"#
        );
    }
}
