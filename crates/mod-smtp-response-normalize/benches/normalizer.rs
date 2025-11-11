use criterion::{Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use mod_text_normalize::normalize;

static GOOG: &str = "Your email has been rate limited because the From: header (RFC5322) in this message isn't aligned with either the authenticated SPF or DKIM organizational domain. To learn more about DMARC alignment, visit  https://support.google.com/a?p=dmarc-alignment  To learn more about Gmail requirements for bulk senders, visit  https://support.google.com/a?p=sender-guidelines. a640c23a62f3a-ab67626ed70si756442266b.465 - gsmtp";

pub fn bench_normalize(c: &mut Criterion) {
    let mut group = c.benchmark_group("normalize longish text");
    group.sampling_mode(SamplingMode::Flat);
    group.throughput(Throughput::Bytes(GOOG.len() as u64));
    group.bench_function(format!("normalize"), |b| {
        b.iter(|| normalize(std::hint::black_box(&GOOG)))
    });
    group.finish();
}

criterion_group!(benches, bench_normalize);
criterion_main!(benches);
