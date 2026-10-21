//! Caches compiled body templates for the HTTP injection API, so that
//! requests that share a template only pay to compile it once.
use kumo_template::{source_is_static, StandaloneTemplate, TemplateDialect};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long a compiled template remains cached after it was compiled
const TEMPLATE_TTL: Duration = Duration::from_secs(600);

static CACHE_ENABLED: AtomicBool = AtomicBool::new(true);

lruttl::declare_cache! {
/// Caches compiled HTTP injection body templates, keyed by a digest
/// of the template dialect, name and source text.
static INJECT_TEMPLATE_CACHE: LruCacheWithTtl<TemplateCacheKey, Arc<StandaloneTemplate>>::new("http_inject_template", 1000);
}

/// SHA-256 of the dialect, name and source. The name and source are
/// length-prefixed so that distinct (name, source) pairs cannot produce
/// the same digest input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TemplateCacheKey([u8; 32]);

impl TemplateCacheKey {
    fn new(dialect: TemplateDialect, name: &str, source: &str) -> Self {
        let dialect_tag: u8 = match dialect {
            TemplateDialect::Jinja => 0,
            TemplateDialect::Static => 1,
            TemplateDialect::Handlebars => 2,
        };
        let mut hasher = Sha256::new();
        hasher.update([dialect_tag]);
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((source.len() as u64).to_le_bytes());
        hasher.update(source.as_bytes());
        Self(hasher.finalize().into())
    }
}

pub fn set_httpinject_template_cache_enabled(enabled: bool) {
    CACHE_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Returns the compiled template for `source`, compiling it only if it is
/// not already cached. Sources with no template syntax, which includes every
/// source in the Static dialect, bypass the cache: they are held as a literal
/// without being compiled, and caching them would only retain memory for
/// content, such as pre-rendered messages, that is unlikely to be seen again.
pub async fn get_or_compile(
    dialect: TemplateDialect,
    name: &str,
    source: &str,
) -> anyhow::Result<Arc<StandaloneTemplate>> {
    get_or_compile_impl(dialect, name, source, CACHE_ENABLED.load(Ordering::Relaxed)).await
}

async fn get_or_compile_impl(
    dialect: TemplateDialect,
    name: &str,
    source: &str,
    cache_enabled: bool,
) -> anyhow::Result<Arc<StandaloneTemplate>> {
    if !cache_enabled || source_is_static(dialect, source) {
        return Ok(Arc::new(StandaloneTemplate::compile(
            dialect, name, source,
        )?));
    }

    let key = TemplateCacheKey::new(dialect, name, source);
    let lookup = INJECT_TEMPLATE_CACHE
        .get_or_try_insert(&key, |_| TEMPLATE_TTL, async {
            StandaloneTemplate::compile(dialect, name, source).map(Arc::new)
        })
        .await
        // Flatten the shared error so that the message is identical
        // regardless of whether the error was freshly computed or cached
        .map_err(|err| anyhow::anyhow!("{err:#}"))?;
    Ok(lookup.item)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn key_depends_on_every_input() {
        let base = TemplateCacheKey::new(TemplateDialect::Jinja, "text_body.txt", "Hi {{ name }}");
        assert_eq!(
            base,
            TemplateCacheKey::new(TemplateDialect::Jinja, "text_body.txt", "Hi {{ name }}")
        );
        assert_ne!(
            base,
            TemplateCacheKey::new(
                TemplateDialect::Handlebars,
                "text_body.txt",
                "Hi {{ name }}"
            )
        );
        // Same text in an html field must not share the entry, as only
        // the html field is escaped
        assert_ne!(
            base,
            TemplateCacheKey::new(TemplateDialect::Jinja, "html_body.html", "Hi {{ name }}")
        );
        assert_ne!(
            base,
            TemplateCacheKey::new(TemplateDialect::Jinja, "text_body.txt", "Hi {{ name }}!")
        );
    }

    #[test]
    fn key_separates_name_and_source() {
        assert_ne!(
            TemplateCacheKey::new(TemplateDialect::Jinja, "ab", "c"),
            TemplateCacheKey::new(TemplateDialect::Jinja, "a", "bc")
        );
    }

    #[tokio::test]
    async fn identical_templates_share_one_compilation() {
        let source = "identical_templates_share_one_compilation {{ name }}";
        let a = get_or_compile(TemplateDialect::Jinja, "text_body.txt", source)
            .await
            .unwrap();
        let b = get_or_compile(TemplateDialect::Jinja, "text_body.txt", source)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn static_and_disabled_bypass_cache() {
        let source = "static_and_disabled_bypass_cache {{ name }}";
        for (dialect, cache_enabled) in [
            (TemplateDialect::Static, true),
            (TemplateDialect::Jinja, false),
            (TemplateDialect::Handlebars, false),
        ] {
            let a = get_or_compile_impl(dialect, "text_body.txt", source, cache_enabled)
                .await
                .unwrap();
            let b = get_or_compile_impl(dialect, "text_body.txt", source, cache_enabled)
                .await
                .unwrap();
            assert!(!Arc::ptr_eq(&a, &b), "{dialect:?} {cache_enabled}");
            let key = TemplateCacheKey::new(dialect, "text_body.txt", source);
            assert!(
                INJECT_TEMPLATE_CACHE.get(&key).is_none(),
                "{dialect:?} {cache_enabled}"
            );
        }
    }

    #[tokio::test]
    async fn template_free_sources_are_not_cached() {
        for (dialect, source) in [
            (
                TemplateDialect::Jinja,
                "template_free_sources_are_not_cached pre-rendered text",
            ),
            // Jinja statement syntax is not Handlebars syntax
            (
                TemplateDialect::Handlebars,
                "template_free_sources_are_not_cached {% if x %}text{% endif %}",
            ),
        ] {
            let a = get_or_compile(dialect, "text_body.txt", source)
                .await
                .unwrap();
            let b = get_or_compile(dialect, "text_body.txt", source)
                .await
                .unwrap();
            assert!(!Arc::ptr_eq(&a, &b), "{dialect:?}");
            let key = TemplateCacheKey::new(dialect, "text_body.txt", source);
            assert!(INJECT_TEMPLATE_CACHE.get(&key).is_none(), "{dialect:?}");
        }
    }

    #[tokio::test]
    async fn any_template_syntax_is_cached() {
        for (dialect, source) in [
            (
                TemplateDialect::Jinja,
                "any_template_syntax_is_cached {{ x }}",
            ),
            (
                TemplateDialect::Jinja,
                "any_template_syntax_is_cached {% if x %}y{% endif %}",
            ),
            (
                TemplateDialect::Jinja,
                "any_template_syntax_is_cached {# c #}",
            ),
            (
                TemplateDialect::Handlebars,
                "any_template_syntax_is_cached {{#if x}}y{{/if}}",
            ),
        ] {
            let a = get_or_compile(dialect, "text_body.txt", source)
                .await
                .unwrap();
            let b = get_or_compile(dialect, "text_body.txt", source)
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&a, &b), "{dialect:?} {source}");
        }
    }

    #[tokio::test]
    async fn syntax_error_message_is_stable_when_cached() {
        let source = "syntax_error_message_is_stable_when_cached {{ name";
        let uncached = StandaloneTemplate::compile(TemplateDialect::Jinja, "text_body.txt", source)
            .unwrap_err();
        let first = get_or_compile(TemplateDialect::Jinja, "text_body.txt", source)
            .await
            .unwrap_err();
        let second = get_or_compile(TemplateDialect::Jinja, "text_body.txt", source)
            .await
            .unwrap_err();
        assert_eq!(format!("{first:#}"), format!("{uncached:#}"));
        assert_eq!(format!("{second:#}"), format!("{uncached:#}"));
    }
}
