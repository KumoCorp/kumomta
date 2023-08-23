use kumo_dkim::canonicalization::Type;
use kumo_dkim::{DkimPrivateKey, ParsedEmail, SignerBuilder};
use chrono::TimeZone;
use rsa::pkcs1::DecodeRsaPrivateKey;
use std::time::Instant;

fn email_text() -> String {
    r#"Subject: subject
From: Sven Sauleau <sven@cloudflare.com>
Subject: This is a very good  subject

Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed nec odio ipsum. Donec maximus faucibus
urna sit amet consequat. Ut a metus ante. Morbi iaculis leo at tellus varius ultricies. Sed
dignissim laoreet lacus ut volutpat. Integer sed dignissim nibh. Etiam congue est quis euismod
luctus. In nec eros eget dolor dapibus bibendum. Quisque in erat et velit lobortis imperdiet id non
dolor. Cras finibus urna tincidunt nisi porta feugiat. Nam facilisis, odio at eleifend lobortis,
diam tellus bibendum urna, sit amet tincidunt lacus neque ut felis. Etiam non sollicitudin arcu,
eget mollis massa. Mauris felis eros, elementum consectetur posuere finibus, porta aliquam sapien.
Suspendisse hendrerit erat ac tortor blandit, sit amet molestie sem ultricies. Sed tempor lorem id
dolor vehicula, non ornare mauris hendrerit. Ut quis venenatis sapien.

Proin sed turpis porttitor, porttitor lorem quis, sagittis lacus. Aenean malesuada vehicula nisi.
Curabitur pulvinar et ex et cursus. Nunc egestas nisi nec urna porta, vel tempor eros ultricies. Ut
gravida est velit, in volutpat quam imperdiet sit amet. Suspendisse risus justo, tristique nec
sodales non, porta eget metus. Nullam malesuada dignissim facilisis. Donec maximus ante faucibus
consequat dignissim. Ut suscipit vel velit a sollicitudin.

Curabitur dictum lorem eget purus tincidunt, id semper velit malesuada. Nunc sollicitudin aliquam
magna vitae luctus. In lacinia, nibh sed pellentesque consectetur, eros mauris molestie nisi, in
vulputate dolor orci egestas massa. Cras odio eros, dignissim aliquet pellentesque ac, luctus vitae
urna. Duis in auctor justo. Integer at lorem volutpat, tempor justo id, congue nisi. Etiam
tincidunt diam eu pellentesque tincidunt. Integer eu dignissim magna. Phasellus molestie gravida
nulla eget blandit. Praesent non eleifend tortor, sed mollis risus. Praesent quis cursus neque, eu
efficitur erat. Aliquam porta metus ut malesuada semper. Cras quis risus eros. Pellentesque
ullamcorper velit diam, et suscipit lacus interdum eu. Fusce ut dui ut mi ullamcorper hendrerit.

Curabitur vulputate leo ac molestie faucibus. Curabitur sit amet condimentum lectus, ut tempor
nibh. Donec id molestie mi, aliquet porta lorem. In non ultricies sapien, non aliquam odio. Nullam
in tellus hendrerit, porttitor mauris eget, finibus enim. Integer scelerisque cursus eros non
eleifend. Nulla vehicula a justo vitae sollicitudin. Etiam volutpat lectus a libero dignissim
sagittis.

Fusce rhoncus, diam quis tincidunt iaculis, nunc est ultricies sapien, vel aliquam leo diam quis
augue. Ut eu tempor nisi. Mauris a ex malesuada, cursus neque id, dignissim quam. Suspendisse odio
nisl, ultrices at ipsum vitae, congue commodo turpis. In porta nunc vitae cursus congue. Donec
suscipit mattis risus non placerat. Sed imperdiet, nisi et laoreet imperdiet, urna felis tristique
ante, non ultricies enim lorem cursus ante. Mauris euismod turpis eu tristique lobortis. Mauris
aliquet eu tortor nec hendrerit. Aliquam in arcu ac erat venenatis pretium at sit amet magna. Lorem
ipsum dolor sit a.
        "#
    .replace("\n", "\r\n")
}

fn main() {
    let email_text = email_text();
    let email = ParsedEmail::parse_bytes(email_text.as_bytes()).unwrap();

    for canon in [Type::Simple, Type::Relaxed] {
        let private_key =
            rsa::RsaPrivateKey::read_pkcs1_pem_file("crates/dkim/test/keys/2022.private").unwrap();
        let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

        let signer = SignerBuilder::new()
            .with_signed_headers(["From", "Subject"])
            .unwrap()
            .with_body_canonicalization(canon)
            .with_header_canonicalization(canon)
            .with_private_key(DkimPrivateKey::Rsa(private_key))
            .with_selector("s20")
            .with_signing_domain("example.com")
            .with_time(time)
            .build()
            .unwrap();

        let start = Instant::now();
        let num_iters = 1_000;
        for _ in 0..num_iters {
            signer.sign(&email).unwrap();
        }
        println!("{canon:?}: Did {num_iters} iters in {:?}", start.elapsed());
    }

    #[cfg(feature = "openssl")]
    for canon in [Type::Simple, Type::Relaxed] {
        let data = std::fs::read("./crates/dkim/test/keys/2022.private").unwrap();
        let pkey = openssl::rsa::Rsa::private_key_from_pem(&data).unwrap();
        let time = chrono::Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 1).unwrap();

        let signer = SignerBuilder::new()
            .with_signed_headers(["From", "Subject"])
            .unwrap()
            .with_body_canonicalization(canon)
            .with_header_canonicalization(canon)
            .with_private_key(DkimPrivateKey::OpenSSLRsa(pkey))
            .with_selector("s20")
            .with_signing_domain("example.com")
            .with_time(time)
            .build()
            .unwrap();

        let start = Instant::now();
        let num_iters = 1_000;
        for _ in 0..num_iters {
            signer.sign(&email).unwrap();
        }
        println!(
            "openssl {canon:?}: Did {num_iters} iters in {:?}",
            start.elapsed()
        );
    }
}
