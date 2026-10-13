use criterion::{criterion_group, criterion_main, Criterion};
use kumo_template::{TemplateDialect, TemplateEngine};
use serde_json::json;

// One key per recipient field and custom header a batch injection request
// typically substitutes.
fn context() -> serde_json::Value {
    json!({
        "name": "James Smythe",
        "to_header": "\"James Smythe\" <user@example.com>",
        "first_name": "James",
        "last_name": "Smythe",
        "company": "Example Corp",
        "unsubscribe_url": "https://example.com/u/abcdef0123456789",
        "tracking_id": "0123456789abcdef0123456789abcdef",
        "campaign": "autumn-2026",
        "list_id": "newsletter-weekly",
        "locale": "en-US",
    })
}

/// Compares rendering a marker-free, static template against a
/// one-substitution dynamic template, for each dialect.
pub fn bench_expand(c: &mut Criterion) {
    let ctx = context();

    for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
        let mut engine = TemplateEngine::with_dialect(dialect);
        // No markers: served as Template::Static.
        engine
            .add_template("static", "text/html; charset=\"utf-8\"")
            .unwrap();
        // The one substitution is evaluated by the engine every render.
        engine.add_template("dynamic", "hello {{ name }}").unwrap();

        let static_template = engine.get_template("static").unwrap();
        let dynamic_template = engine.get_template("dynamic").unwrap();
        assert!(static_template.is_static());
        assert!(!dynamic_template.is_static());

        let mut group = c.benchmark_group(format!("{dialect:?}"));
        group.bench_function("static_literal", |b| {
            b.iter(|| static_template.render(std::hint::black_box(&ctx)).unwrap())
        });
        group.bench_function("dynamic_engine", |b| {
            b.iter(|| dynamic_template.render(std::hint::black_box(&ctx)).unwrap())
        });
        group.finish();
    }
}

/// Compares the time to render TEMPLATES templates with Template::render,
/// which converts the substitution values on every call, against the time
/// with Template::render_prepared, reusing a context converted once with
/// TemplateEngine::prepare_context.
pub fn bench_prepared_context(c: &mut Criterion) {
    let ctx = context();
    // One message body plus several header values, the number of templates a
    // caller typically renders against the same substitution values.
    const TEMPLATES: usize = 6;

    for dialect in [TemplateDialect::Jinja, TemplateDialect::Handlebars] {
        let mut engine = TemplateEngine::with_dialect(dialect);
        for i in 0..TEMPLATES {
            engine
                .add_template(format!("t{i}"), "hello {{ name }}")
                .unwrap();
        }
        let templates: Vec<_> = (0..TEMPLATES)
            .map(|i| engine.get_template(&format!("t{i}")).unwrap())
            .collect();

        let mut group = c.benchmark_group(format!("{dialect:?}/{TEMPLATES}-renders"));
        group.bench_function("per_call_serialize", |b| {
            b.iter(|| {
                for t in &templates {
                    std::hint::black_box(t.render(&ctx).unwrap());
                }
            })
        });
        group.bench_function("prepared_once", |b| {
            b.iter(|| {
                let prepared = engine.prepare_context(&ctx).unwrap();
                for t in &templates {
                    std::hint::black_box(t.render_prepared(&prepared).unwrap());
                }
            })
        });
        group.finish();
    }
}

criterion_group!(benches, bench_expand, bench_prepared_context);
criterion_main!(benches);
