#![cfg(test)]
use crate::{
    dns, verify_email_with_resolver, DKIMError, DkimPrivateKey, ParsedEmail, SignerBuilder,
};
use chrono::TimeZone;
use futures::future::BoxFuture;
use mailparsing::AuthenticationResult;
use regex::Regex;
use std::collections::HashMap;

fn dkim_record() -> String {
    let data = std::fs::read_to_string("./test/keys/2022.txt").unwrap();
    let re = Regex::new(r#"".*""#).unwrap();

    let mut out = "".to_owned();
    for m in re.find_iter(&data) {
        out += &m.as_str().replace('\"', "");
    }
    out
}

fn sign(domain: &str, raw_email: &str) -> String {
    let email = ParsedEmail::parse(raw_email).unwrap();

    let private_key = DkimPrivateKey::rsa_key_file("./test/keys/2022.private").unwrap();
    let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

    let signer = SignerBuilder::new()
        .with_signed_headers(["From", "Subject"])
        .unwrap()
        .with_private_key(private_key)
        .with_selector("2022")
        .with_signing_domain(domain)
        .with_time(time)
        .build()
        .unwrap();
    let header = signer.sign(&email).unwrap();

    let private_key = DkimPrivateKey::rsa_key_file("./test/keys/2022.private").unwrap();
    let signer = SignerBuilder::new()
        .with_signed_headers(["From", "Subject"])
        .unwrap()
        .with_private_key(private_key)
        .with_selector("2022")
        .with_signing_domain(format!("not.{domain}"))
        .with_time(time)
        .build()
        .unwrap();
    let header2 = signer.sign(&email).unwrap();

    let private_key = DkimPrivateKey::rsa_key_file("./test/keys/2022.private").unwrap();
    let signer = SignerBuilder::new()
        .with_signed_headers(["From", "Subject"])
        .unwrap()
        .with_private_key(private_key)
        .with_selector("bogus-selector")
        .with_signing_domain(domain)
        .with_time(time)
        .build()
        .unwrap();
    let header3 = signer.sign(&email).unwrap();

    format!("{header}\r\n{header2}\r\n{header3}\r\n{raw_email}")
}

async fn verify(
    resolver: &dyn dns::Lookup,
    from_domain: &str,
    raw_email: &str,
) -> Vec<AuthenticationResult> {
    let email = ParsedEmail::parse(raw_email).unwrap();

    verify_email_with_resolver(from_domain, &email, resolver)
        .await
        .unwrap()
}

struct TestResolver {
    db: HashMap<&'static str, String>,
}
impl dns::Lookup for TestResolver {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<String>, DKIMError>> {
        let res = if let Some(value) = self.db.get(name) {
            Ok(vec![value.to_string()])
        } else {
            Err(DKIMError::KeyUnavailable(format!(
                "failed to resolve {name}"
            )))
        };
        Box::pin(async move { res })
    }
}

impl TestResolver {
    fn new<I: IntoIterator<Item = (&'static str, String)>>(iter: I) -> Self {
        Self {
            db: HashMap::from_iter(iter),
        }
    }
}

#[tokio::test]
async fn test_roundtrip() {
    let resolver = TestResolver::new([
        ("2022._domainkey.cloudflare.com", dkim_record()),
        ("2022._domainkey.not.cloudflare.com", dkim_record()),
    ]);
    let from_domain = "cloudflare.com";

    {
        let email = r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

Hello Alice
"#
        .replace("\n", "\r\n");

        let signed_email = sign(from_domain, &email);
        eprintln!("input email:\n{email:?}");
        eprintln!("signed email:\n{signed_email:?}");
        let res = verify(&resolver, from_domain, &signed_email).await;
        k9::snapshot!(
            res,
            r#"
[
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "pass",
        reason: None,
        props: {
            "header.a": "rsa-sha256",
            "header.b": "vHLsP0n+",
            "header.d": "cloudflare.com",
            "header.i": "@cloudflare.com",
            "header.s": "2022",
        },
    },
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "policy",
        reason: Some(
            "mail-from-mismatch-signing-domain",
        ),
        props: {
            "header.a": "rsa-sha256",
            "header.b": "fdUa++8n",
            "header.d": "not.cloudflare.com",
            "header.i": "@not.cloudflare.com",
            "header.s": "2022",
            "policy.dkim-rules": "mail-from-mismatch-signing-domain",
        },
    },
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "temperror",
        reason: Some(
            "key unavailable: failed to resolve bogus-selector._domainkey.cloudflare.com",
        ),
        props: {
            "header.a": "rsa-sha256",
            "header.b": "U0HRrJ9u",
            "header.d": "cloudflare.com",
            "header.i": "@cloudflare.com",
            "header.s": "bogus-selector",
        },
    },
]
"#
        );
    }

    {
        let email = r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>

.Hello Alice...
.
...
"#
        .replace("\n", "\r\n");

        let signed_email = sign(from_domain, &email);
        let res = verify(&resolver, from_domain, &signed_email).await;
        k9::snapshot!(
            res,
            r#"
[
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "pass",
        reason: None,
        props: {
            "header.a": "rsa-sha256",
            "header.b": "qSowczhl",
            "header.d": "cloudflare.com",
            "header.i": "@cloudflare.com",
            "header.s": "2022",
        },
    },
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "policy",
        reason: Some(
            "mail-from-mismatch-signing-domain",
        ),
        props: {
            "header.a": "rsa-sha256",
            "header.b": "UZw1wwBY",
            "header.d": "not.cloudflare.com",
            "header.i": "@not.cloudflare.com",
            "header.s": "2022",
            "policy.dkim-rules": "mail-from-mismatch-signing-domain",
        },
    },
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "temperror",
        reason: Some(
            "key unavailable: failed to resolve bogus-selector._domainkey.cloudflare.com",
        ),
        props: {
            "header.a": "rsa-sha256",
            "header.b": "GI3Q15Rv",
            "header.d": "cloudflare.com",
            "header.i": "@cloudflare.com",
            "header.s": "bogus-selector",
        },
    },
]
"#
        );
    }

    {
        let email = r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>
Mime-Version: 1.0
Content-Type: multipart/alternative; boundary=2c637dd08e3ccac9b9425780c2e07981cb322e7feed138813fb1ab054047

--2c637dd08e3ccac9b9425780c2e07981cb322e7feed138813fb1ab054047
Content-Transfer-Encoding: 7bit
Content-Type: text/plain; charset=ascii

text here
--2c637dd08e3ccac9b9425780c2e07981cb322e7feed138813fb1ab054047
Content-Transfer-Encoding: quoted-printable
Content-Type: text/html; charset=ascii

<!doctype html><html xmlns=3D"http://www.w3.org/1999/xhtml" xmlns:v=3D"urn:=
schemas-microsoft-com:vml" xmlns:o=3D"urn:schemas-microsoft-com:office:offi=
ce"><head><title></title><!--[if !mso]><!-- --><meta http-equiv=3D"X-UA-Com=
patible" content=3D"IE=3Dedge"><!--<![endif]--><meta http-equiv=3D"Content-=
Type" content=3D"text/html; charset=3DUTF-8"><meta name=3D"viewport" conten=
t=3D"width=3Ddevice-width,initial-scale=3D1"><style type=3D"text/css">#outl=
ook a { padding:0; }
          .ReadMsgBody { width:100%; }
          .ExternalClass { width:100%; }
      div.footer-text a {
        color: #3498db;
      }  td {
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Roboto=
', 'Oxygen', 'Ubuntu', 'Fira Sans', 'Droid Sans', 'Helvetica Neue', sans-se=
rif !important;
      }</style></head><body style=3D"font-size: 16px; line-height: 24px; fo=
nt-weight: normal; font-style: normal; background-color: #fbfbfb;"><div sty=
le=3D"display:none;font-size:1px;color:#ffffff;line-height:1px;max-height:0=
px;max-width:0px;opacity:0;overflow:hidden;"> Completed - No components aff=
ected - The scheduled maintenance has been completed. &zwnj;&nbsp;&zwnj;&nb=
sp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;=
&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&zw=
nj;&nbsp;&zwnj;&nbsp;&zwnj;&nbsp;&nbsp;&zwnj;&nbsp;</div>=
<div style=3D"background-color:#fbfbfb;"><!--[if mso | IE]><table align=3D"=
center" border=3D"0" cellpadding=3D"0" cellspacing=3D"0" class=3D"header-sp=
acing-outlook" style=3D"width:600px;" width=3D"600" ><tr><td style=3D"line-=
height:0px;font-size:0px;mso-line-height-rule:exactly;"><![endif]--><div cl=
ass=3D"header-spacing" style=3D"Margin:0px auto;max-width:600px;"><table al=
ign=3D"center" border=3D"0" cellpadding=3D"0" cellspacing=3D"0" role=3D"pre=
sentation" style=3D"width:100%;">

--2c637dd08e3ccac9b9425780c2e07981cb322e7feed138813fb1ab054047--
"#.replace("\n", "\r\n");

        let signed_email = sign(from_domain, &email);
        let res = verify(&resolver, from_domain, &signed_email).await;
        k9::snapshot!(
            res,
            r#"
[
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "pass",
        reason: None,
        props: {
            "header.a": "rsa-sha256",
            "header.b": "h60+VEgs",
            "header.d": "cloudflare.com",
            "header.i": "@cloudflare.com",
            "header.s": "2022",
        },
    },
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "policy",
        reason: Some(
            "mail-from-mismatch-signing-domain",
        ),
        props: {
            "header.a": "rsa-sha256",
            "header.b": "WzP4DTuC",
            "header.d": "not.cloudflare.com",
            "header.i": "@not.cloudflare.com",
            "header.s": "2022",
            "policy.dkim-rules": "mail-from-mismatch-signing-domain",
        },
    },
    AuthenticationResult {
        method: "dkim",
        method_version: None,
        result: "temperror",
        reason: Some(
            "key unavailable: failed to resolve bogus-selector._domainkey.cloudflare.com",
        ),
        props: {
            "header.a": "rsa-sha256",
            "header.b": "WKnfSsMb",
            "header.d": "cloudflare.com",
            "header.i": "@cloudflare.com",
            "header.s": "bogus-selector",
        },
    },
]
"#
        );
    }
}
