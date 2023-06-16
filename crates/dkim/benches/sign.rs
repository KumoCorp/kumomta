use cfdkim::canonicalization::Type;
use cfdkim::{DkimPrivateKey, ParsedEmail, SignerBuilder};
use chrono::TimeZone;
use criterion::{black_box, criterion_group, criterion_main, Criterion, SamplingMode, Throughput};
use rsa::pkcs1::DecodeRsaPrivateKey;

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

pub fn criterion_benchmark(c: &mut Criterion) {
    let email_text = email_text();
    let email = ParsedEmail::parse_bytes(email_text.as_bytes()).unwrap();

    for canon in [Type::Simple, Type::Relaxed] {
        let private_key =
            rsa::RsaPrivateKey::read_pkcs1_pem_file("./test/keys/2022.private").unwrap();
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

        let mut group = c.benchmark_group("cfdkim signing");
        group.sampling_mode(SamplingMode::Flat);
        group.throughput(Throughput::Bytes(email_text.len() as u64));
        group.bench_function(&format!("sign {canon:?}"), |b| {
            b.iter(|| signer.sign(black_box(&email)).unwrap())
        });
        group.finish();
    }

    #[cfg(feature = "openssl")]
    for canon in [Type::Simple, Type::Relaxed] {
        let data = std::fs::read("./test/keys/2022.private").unwrap();
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

        let mut group = c.benchmark_group("cfdkim openssl signing");
        group.sampling_mode(SamplingMode::Flat);
        group.throughput(Throughput::Bytes(email_text.len() as u64));
        group.bench_function(&format!("sign {canon:?}"), |b| {
            b.iter(|| signer.sign(black_box(&email)).unwrap())
        });
        group.finish();
    }
}

pub fn mail_auth_benchmark(c: &mut Criterion) {
    let email_text = email_text();

    use mail_auth::common::crypto::{RsaKey, Sha256};
    use mail_auth::dkim::{Canonicalization, DkimSigner};

    for canon in [Canonicalization::Simple, Canonicalization::Relaxed] {
        let key_data = std::fs::read_to_string("./test/keys/2022.private").unwrap();
        let key = RsaKey::<Sha256>::from_rsa_pem(&key_data).unwrap();

        let signer = DkimSigner::from_key(key)
            .domain("example.com")
            .selector("s20")
            .headers(["From", "Subject"])
            .header_canonicalization(canon)
            .body_canonicalization(canon);

        let mut group = c.benchmark_group("mail-auth signing");
        group.sampling_mode(SamplingMode::Flat);
        group.throughput(Throughput::Bytes(email_text.len() as u64));
        group.bench_function(&format!("sign {canon:?}"), |b| {
            b.iter(|| signer.sign(black_box(email_text.as_bytes())).unwrap())
        });
        group.finish();
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_group!(mail_auth, mail_auth_benchmark);
criterion_main!(benches, mail_auth);
