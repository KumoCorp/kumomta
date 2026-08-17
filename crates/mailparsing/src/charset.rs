use charset_normalizer_rs::Encoding;
use std::collections::BTreeMap;
use std::sync::RwLock;

/// Well-known names that are byte-compatible with a supported encoding but are
/// absent from the standard label table. Pre-registered so common mail decodes
/// without configuration; runtime aliases can add more or override these.
const BUILTIN_ALIASES: &[(&str, &str)] = &[
    ("ms949", "euc-kr"),
    ("cp949", "euc-kr"),
    ("uhc", "euc-kr"),
    ("x-windows-949", "euc-kr"),
    ("cp932", "shift_jis"),
    ("cp936", "gbk"),
    ("ms936", "gbk"),
    ("cp950", "big5"),
    ("iso-8859-8-i", "iso-8859-8"),
];

/// Runtime-registered aliases, populated via `kumo.add_charset_alias`.
pub static CHARSET_ALIASES: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// Resolve a charset name. Runtime aliases take precedence, then the standard
/// label table, then the built-in aliases.
pub fn resolve_charset(name: &str) -> Option<&'static Encoding> {
    let lower = name.to_ascii_lowercase();

    if let Some(target) = CHARSET_ALIASES
        .read()
        .expect("charset alias registry lock poisoned")
        .get(&lower)
    {
        return Encoding::by_name(target);
    }

    if let Some(enc) = Encoding::by_name(name) {
        return Some(enc);
    }

    BUILTIN_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .and_then(|(_, target)| Encoding::by_name(target))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn builtin_alias_resolution() {
        let ms949 = resolve_charset("MS949").expect("ms949 is built-in");
        assert_eq!(ms949.name(), Encoding::by_name("euc-kr").unwrap().name());
    }

    #[test]
    fn runtime_alias_resolution() {
        assert!(resolve_charset("x-test-custom").is_none());
        CHARSET_ALIASES
            .write()
            .unwrap()
            .insert("x-test-custom".to_string(), "euc-kr".to_string());
        let via_alias = resolve_charset("X-Test-Custom").expect("alias should resolve");
        assert_eq!(
            via_alias.name(),
            Encoding::by_name("euc-kr").unwrap().name()
        );
    }
}
