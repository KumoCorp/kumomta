# kumo-dkim

> DKIM ([RFC6376]) implementation

## Features

### Verifying email signatures

Example:
```rust
let res: DKIMResult = kumo_dkim::verify_email(&from_domain, &parsed_email).await?;

if let Some(err) = &res.error() {
  error!(logger, "dkim verify fail: {}", err);
}

println!("dkim={}", res.with_detail());
```

### Signing an email

Example:
```rust
let private_key =
    rsa::RsaPrivateKey::read_pkcs1_pem_file(Path::new("./test/keys/2022.private"))?;

let signer = SignerBuilder::new()
    .with_signed_headers(["From", "Subject"])?
    .with_private_key(private_key)
    .with_selector("2020")
    .with_signing_domain("example.com")
    .build()?;
let signature = signer.sign(&email)?;

println!("{}", signature); // DKIM-Signature: ...
```

See the SignerBuilder object documentation for more information.

## Generate a test DKIM key

Using [OpenDKIM]:
```
opendkim-genkey \
    --testmode \
    --domain=example.com \
    --selector=2022 \
    --nosubdomains
```

[RFC5322]: https://datatracker.ietf.org/doc/html/rfc5322
[RFC6376]: https://datatracker.ietf.org/doc/html/rfc6376
[OpenDKIM]: http://www.opendkim.org/
