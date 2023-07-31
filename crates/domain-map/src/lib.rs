//! This module provides a simple datastructure that can store
//! values associated with a domain name style key.
//! Wildcard keys are supported.
use config::get_or_create_sub_module;
use mlua::prelude::LuaUserData;
use mlua::{FromLua, Lua, MetaMethod, UserDataMethods};
use mod_memoize::CacheValue;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;

#[derive(Clone)]
struct Node<V: Clone> {
    value: Option<V>,
    label: String,
    children: HashMap<String, Self>,
}

impl<V: Debug + Clone> Debug for Node<V> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("Node")
            .field("value", &self.value)
            .field("label", &self.label)
            .field("children", &self.children)
            .finish()
    }
}

/// A DomainMap is conceptually similar to a HashMap. The differences
/// are that the keys are always domain name strings like "example.com"
/// and that a lookup that doesn't have an exact match in the map is
/// allowed to resolve through a wildcard entry, such as "*.example.com",
/// if one has been inserted.
/// A lookup for "example.com" will not match the wildcard "*.example.com"
/// because it has fewer segments than the wildcard entry.
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(from = "BTreeMap<String, V>", into = "BTreeMap<String,V>")]
pub struct DomainMap<V: Clone> {
    top: HashMap<String, Node<V>>,
}

impl LuaUserData for DomainMap<CacheValue> {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        mod_memoize::Memoized::impl_memoize(methods);
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            if let Some(value) = this.get(&key) {
                let value = value.as_lua(lua)?;
                Ok(Some(value))
            } else {
                Ok(None)
            }
        });
        methods.add_meta_method_mut(
            MetaMethod::NewIndex,
            |lua, this, (key, value): (String, mlua::Value)| {
                let value = CacheValue::from_lua(value, lua)?;
                this.insert(&key, value);
                Ok(())
            },
        );
    }
}

impl<V: Debug + Clone> Debug for DomainMap<V> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("DomainMap")
            .field("top", &self.top)
            .finish()
    }
}

impl<V: Clone> DomainMap<V> {
    pub fn new() -> Self {
        Self {
            top: HashMap::new(),
        }
    }

    pub fn insert(&mut self, pattern: &str, value: V) {
        let mut current = &mut self.top;
        let mut iter = pattern.rsplit('.').peekable();
        while let Some(seg) = iter.next() {
            let node = current.entry(seg.to_string()).or_insert_with(|| Node {
                value: None,
                label: seg.to_string(),
                children: HashMap::new(),
            });

            if iter.peek().is_none() {
                // No further segments: this is where we set our value
                node.value.replace(value);
                return;
            }
            current = &mut node.children;
        }
    }

    pub fn get(&self, pattern: &str) -> Option<&V> {
        let mut current = &self.top;
        let mut iter = pattern.rsplit('.').peekable();
        while let Some(seg) = iter.next() {
            match current.get(seg) {
                Some(node) => {
                    if iter.peek().is_none() {
                        // This node holds our exact match
                        return node.value.as_ref();
                    }
                    current = &node.children;
                    continue;
                }
                None => {
                    // No exact match; see if there is a wildcard
                    let wild = current.get("*")?;
                    return wild.value.as_ref();
                }
            }
        }
        None
    }
}

impl<V: Clone> From<BTreeMap<String, V>> for DomainMap<V> {
    fn from(map: BTreeMap<String, V>) -> Self {
        let mut result = DomainMap::new();
        for (k, v) in map {
            result.insert(&k, v);
        }
        result
    }
}

fn walk<'a, V: Clone>(
    nodes: &'a HashMap<String, Node<V>>,
    stack: &mut Vec<&'a str>,
    result: &mut BTreeMap<String, V>,
) {
    for (key, value) in nodes {
        stack.insert(0, key);
        if let Some(v) = &value.value {
            result.insert(stack.join("."), v.clone());
        }
        walk(&value.children, stack, result);
        stack.remove(0);
    }
}

impl<V: Clone> From<DomainMap<V>> for BTreeMap<String, V> {
    fn from(map: DomainMap<V>) -> Self {
        let mut result = BTreeMap::new();
        let mut stack = vec![];
        walk(&map.top, &mut stack, &mut result);

        result
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let dmap_mod = get_or_create_sub_module(lua, "domain_map")?;

    dmap_mod.set(
        "new",
        lua.create_function(|lua, value: Option<HashMap<String, mlua::Value>>| {
            let mut dmap: DomainMap<mod_memoize::CacheValue> = DomainMap::new();

            if let Some(value) = value {
                for (k, v) in value {
                    let v = CacheValue::from_lua(v, lua)?;
                    dmap.insert(&k, v);
                }
            }

            Ok(dmap)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let mut map: DomainMap<u32> = DomainMap::new();
        map.insert("*.example.com", 42);
        map.insert("example.com", 24);
        map.insert("omg.wtf.woot.example.com", 128);
        println!("{map:#?}");

        assert_eq!(map.get("foo.com"), None);
        assert_eq!(map.get("example.com"), Some(&24));
        assert_eq!(map.get("lemon.example.com"), Some(&42));
        assert_eq!(map.get("lemon.cake.example.com"), Some(&42));
        assert_eq!(map.get("woot.example.com"), None);
        assert_eq!(map.get("wtf.woot.example.com"), None);
        assert_eq!(map.get("omg.wtf.woot.example.com"), Some(&128));

        let serialized: BTreeMap<_, _> = map.into();
        k9::snapshot!(
            &serialized,
            r#"
{
    "*.example.com": 42,
    "example.com": 24,
    "omg.wtf.woot.example.com": 128,
}
"#
        );

        let round_trip: DomainMap<_> = serialized.into();
        assert_eq!(round_trip.get("lemon.example.com"), Some(&42));

        let serialized_again: BTreeMap<_, _> = round_trip.into();
        k9::snapshot!(
            &serialized_again,
            r#"
{
    "*.example.com": 42,
    "example.com": 24,
    "omg.wtf.woot.example.com": 128,
}
"#
        );
    }
}
