use config::{any_err, get_or_create_sub_module};
use mlua::prelude::LuaUserData;
use mlua::{FromLua, Lua, MetaMethod, UserDataMethods};
use mod_memoize::CacheValue;
use regex::{RegexSet, RegexSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "RegexSetMapBuilder<V>", into = "RegexSetMapBuilder<V>")]
pub struct RegexSetMap<V: Clone> {
    set: RegexSet,
    pattern_to_value: Vec<V>,
}

impl<V: Clone> RegexSetMap<V> {
    pub fn lookup(&self, subject: &str) -> Option<&V> {
        self.set
            .matches(subject)
            .into_iter()
            .next()
            .and_then(|idx| self.pattern_to_value.get(idx))
    }
}

impl<V: Clone> From<RegexSetMap<V>> for RegexSetMapBuilder<V> {
    fn from(map: RegexSetMap<V>) -> Self {
        let patterns = map.set.patterns().to_vec();
        RegexSetMapBuilder {
            patterns,
            pattern_to_value: map.pattern_to_value,
        }
    }
}

impl<V: Clone> TryFrom<RegexSetMapBuilder<V>> for RegexSetMap<V> {
    type Error = String;

    fn try_from(builder: RegexSetMapBuilder<V>) -> Result<RegexSetMap<V>, String> {
        builder.build()
    }
}

#[derive(Serialize, Deserialize)]
pub struct RegexSetMapBuilder<V: Clone> {
    patterns: Vec<String>,
    pattern_to_value: Vec<V>,
}

impl<V: Clone> RegexSetMapBuilder<V> {
    pub fn new() -> Self {
        Self {
            patterns: vec![],
            pattern_to_value: vec![],
        }
    }

    pub fn add_rule<S: Into<String>>(&mut self, rule: S, value: V) {
        self.patterns.push(rule.into());
        self.pattern_to_value.push(value);
    }

    pub fn build(mut self) -> Result<RegexSetMap<V>, String> {
        self.patterns.shrink_to_fit();
        self.pattern_to_value.shrink_to_fit();

        let set = RegexSetBuilder::new(self.patterns)
            .build()
            .map_err(|err| format!("compiling rules: {err:#}"))?;
        Ok(RegexSetMap {
            set,
            pattern_to_value: self.pattern_to_value,
        })
    }
}

impl LuaUserData for RegexSetMap<CacheValue> {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        mod_memoize::Memoized::impl_memoize(methods);
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            if let Some(value) = this.lookup(&key) {
                let value = value.as_lua(lua)?;
                Ok(Some(value))
            } else {
                Ok(None)
            }
        });
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let module = get_or_create_sub_module(lua, "regex_set_map")?;

    module.set(
        "new",
        lua.create_function(|lua, value: Option<HashMap<String, mlua::Value>>| {
            let mut builder: RegexSetMapBuilder<CacheValue> = RegexSetMapBuilder::new();

            if let Some(value) = value {
                for (k, v) in value {
                    let v = CacheValue::from_lua(v, lua)?;
                    builder.add_rule(&k, v);
                }
            }

            builder.build().map_err(any_err)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_basic_mapping() {
        let mut builder = RegexSetMapBuilder::new();
        builder.add_rule(
            "4\\.2\\.2 The email account that you tried to reach is over quota\\.",
            500,
        );
        builder.add_rule("4\\.2\\.1 <.+>: Recipient address rejected: this mailbox is inactive and has been disabled", 501);
        builder.add_rule("4\\.1\\.1 <.*> 4.2.2 mailbox full\\.", 502);
        let mapper = builder.build().unwrap();

        let corpus = &[
            ("400 4.2.2 The email account that you tried to reach is over quota", None),
            ("400 4.2.2 The email account that you tried to reach is over quota.", Some(500)),
            ("400 4.2.1 <foo>: Recipient address rejected: this mailbox is inactive and has been disabled", Some(501)),
            ("400 4.1.1 <bar> 4.2.2 mailbox full.", Some(502)),
        ];

        for &(input, output) in corpus {
            assert_eq!(
                mapper.lookup(input),
                output.as_ref(),
                "expected {input} -> {output:?}"
            );
        }
    }
}
