// The heart of this CidrMap implementation is derived from rust-bitstring-trees
// which is Copyright (c) 2017 Stefan Bühler and used here under the terms
// of its MIT License.
// The modifications made here are to remove the use of unsafe code and to
// expose the ability to search, rather than simply iterate, the underlying
// radix tree.
use bitstring::BitString;
pub use cidr::{AnyIpCidr, IpCidr};
#[cfg(feature = "lua")]
use config::{any_err, get_or_create_sub_module};
#[cfg(feature = "lua")]
use mlua::prelude::LuaUserData;
#[cfg(feature = "lua")]
use mlua::{FromLua, Lua, MetaMethod, UserDataMethods};
#[cfg(feature = "lua")]
use mod_memoize::CacheValue;
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::Debug;
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Clone, PartialEq)]
pub struct CidrMap<V>
where
    V: Clone,
{
    root: Option<Node<V>>,
}

impl<V> Debug for CidrMap<V>
where
    V: Clone + Debug,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        use std::fmt::DebugMap;
        let mut map = fmt.debug_map();

        fn walk<V: Clone + Debug>(node: &Node<V>, map: &mut DebugMap) {
            match node {
                Node::InnerNode(inner) => {
                    walk(&inner.children.left, map);
                    walk(&inner.children.right, map);
                }
                Node::Leaf(leaf) => {
                    map.key(&leaf.key.to_string());
                    map.value(&leaf.value);
                }
            }
        }

        if let Some(root) = &self.root {
            walk(root, &mut map);
        }

        map.finish()
    }
}

struct MapVis<T>
where
    T: Clone + PartialEq,
{
    map: CidrMap<T>,
}

impl<'de, T> Visitor<'de> for MapVis<T>
where
    T: Clone + PartialEq + Deserialize<'de>,
{
    type Value = CidrMap<T>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a CidrMap")
    }

    fn visit_map<M>(mut self, mut access: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        while let Some((key, value)) = access.next_entry()? {
            self.map.insert(key, value);
        }

        Ok(self.map)
    }
}

impl<'de, V> Deserialize<'de> for CidrMap<V>
where
    V: Clone + PartialEq + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(MapVis {
            map: CidrMap::new(),
        })
    }
}

impl<V> Serialize for CidrMap<V>
where
    V: Clone + PartialEq + Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        for (k, v) in self.iter() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

/// Nodes of a CidrMap can be either an InnerNode (with two children)
/// or a leaf node.
#[derive(Debug, Clone, PartialEq)]
pub enum Node<V>
where
    V: Clone,
{
    /// Inner node
    InnerNode(InnerNode<V>),
    /// Leaf node
    Leaf(Leaf<V>),
}

/// Leaf nodes represent prefixes part of the set
#[derive(Clone, Debug, PartialEq)]
pub struct Leaf<V>
where
    V: Clone,
{
    pub key: AnyIpCidr,
    pub value: V,
}

/// Inner node with two direct children.
#[derive(Clone, Debug, PartialEq)]
pub struct InnerNode<V>
where
    V: Clone,
{
    pub(crate) key: AnyIpCidr,
    pub(crate) children: Box<Children<V>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Children<V>
where
    V: Clone,
{
    pub(crate) left: Node<V>,
    pub(crate) right: Node<V>,
}

impl<V> InnerNode<V>
where
    V: Clone,
{
    pub fn key(&self) -> &AnyIpCidr {
        &self.key
    }

    pub fn pick_side<'a>(&'a self, subkey: &AnyIpCidr) -> &'a Node<V> {
        if subkey.get(self.key.len()) {
            &self.children.right
        } else {
            &self.children.left
        }
    }

    pub fn pick_side_mut<'a>(&'a mut self, subkey: &AnyIpCidr) -> &'a mut Node<V> {
        if subkey.get(self.key.len()) {
            &mut self.children.right
        } else {
            &mut self.children.left
        }
    }

    pub fn left(&self) -> &Node<V> {
        &self.children.left
    }

    pub fn right(&self) -> &Node<V> {
        &self.children.right
    }
}

impl<V> Node<V>
where
    V: Clone,
{
    fn new_leaf(key: AnyIpCidr, value: V) -> Self {
        Self::Leaf(Leaf { key, value })
    }

    fn new_children_unknown_order(
        shared_prefix_len: usize,
        a: Node<V>,
        b: Node<V>,
    ) -> Box<Children<V>> {
        let a_right = a.key().get(shared_prefix_len);
        assert_eq!(!a_right, b.key().get(shared_prefix_len));
        if a_right {
            Box::new(Children { left: b, right: a })
        } else {
            Box::new(Children { left: a, right: b })
        }
    }

    fn new_inner_unknown_order(shared_prefix_len: usize, a: Node<V>, b: Node<V>) -> Node<V> {
        let mut key = a.key().clone();
        key.clip(shared_prefix_len);
        Node::InnerNode(InnerNode {
            key,
            children: Self::new_children_unknown_order(shared_prefix_len, a, b),
        })
    }

    /// The longest shared prefix of all nodes in this sub tree.
    pub fn key(&self) -> &AnyIpCidr {
        match *self {
            Node::Leaf(ref leaf) => &leaf.key,
            Node::InnerNode(ref inner) => &inner.key,
        }
    }

    fn leaf_ref(&self) -> Option<&Leaf<V>> {
        match *self {
            Node::Leaf(ref leaf) => Some(leaf),
            _ => None,
        }
    }

    /// convert self node to leaf with key clipped to key_len and given
    /// value
    fn convert_leaf(&mut self, key_len: usize, value: V) {
        *self = match self {
            Node::Leaf(leaf) => {
                let mut leaf = leaf.clone();
                leaf.key.clip(key_len);
                leaf.value = value;
                Node::Leaf(leaf)
            }
            Node::InnerNode(inner) => {
                let mut key = inner.key;
                key.clip(key_len);
                Self::new_leaf(key, value)
            }
        };
    }

    fn insert_uncompressed(&mut self, key: AnyIpCidr, value: V)
    where
        V: Clone,
    {
        let (self_key_len, shared_prefix_len) = {
            let key_ref = self.key();
            (key_ref.len(), key_ref.shared_prefix_len(&key))
        };

        if shared_prefix_len == key.len() {
            // either key == self.key, or key is a prefix of self.key
            // => replace subtree
            self.convert_leaf(shared_prefix_len, value);
        } else if shared_prefix_len < self_key_len {
            debug_assert!(shared_prefix_len < key.len());
            // need to split path to current node; requires new parent
            *self = Self::new_inner_unknown_order(
                shared_prefix_len,
                self.clone(),
                Self::new_leaf(key, value),
            );
        } else {
            debug_assert!(shared_prefix_len == self_key_len);
            debug_assert!(shared_prefix_len < key.len());
            // new key below in tree
            match *self {
                Node::Leaf(_) => {
                    // linear split of path down to leaf
                    let old_value = self.leaf_ref().unwrap().value.clone();
                    let mut new_node = Self::new_leaf(key.clone(), value);
                    for l in (shared_prefix_len..key.len()).rev() {
                        let mut other_key = key.clone();
                        other_key.clip(l + 1);
                        other_key.flip(l);
                        new_node = Self::new_inner_unknown_order(
                            l,
                            new_node,
                            Self::new_leaf(other_key, old_value.clone()),
                        );
                    }
                    *self = new_node;
                }
                Node::InnerNode(ref mut inner) => {
                    inner.pick_side_mut(&key).insert_uncompressed(key, value);
                }
            }
        }
    }

    fn insert(&mut self, key: AnyIpCidr, value: V)
    where
        V: Clone + PartialEq,
    {
        let (self_key_len, shared_prefix_len) = {
            let key_ref = self.key();
            (key_ref.len(), key_ref.shared_prefix_len(&key))
        };

        if shared_prefix_len == key.len() {
            // either key == self.key, or key is a prefix of self.key
            // => replace subtree
            self.convert_leaf(shared_prefix_len, value);
        // no need to compress
        } else if shared_prefix_len < self_key_len {
            debug_assert!(shared_prefix_len < key.len());
            if shared_prefix_len + 1 == self_key_len && shared_prefix_len + 1 == key.len() {
                if let Node::Leaf(ref mut this) = *self {
                    if this.value == value {
                        // we'd split this, and compress it below.
                        // shortcut the allocations here
                        this.key.clip(shared_prefix_len);
                        return; // no need split path
                    }
                }
            }

            // need to split path to current node; requires new parent
            *self = Self::new_inner_unknown_order(
                shared_prefix_len,
                self.clone(),
                Self::new_leaf(key, value),
            );
        // no need to compress - shortcut check above would
        // have found it
        } else {
            debug_assert!(shared_prefix_len == self_key_len);
            debug_assert!(shared_prefix_len < key.len());
            // new key below in tree
            match *self {
                Node::Leaf(_) => {
                    // linear split of path down to leaf
                    let new_node = {
                        let old_value = &self.leaf_ref().unwrap().value;
                        if *old_value == value {
                            // below in tree, but same value - nothing new
                            return;
                        }
                        let mut new_node = Self::new_leaf(key.clone(), value);
                        for l in (shared_prefix_len..key.len()).rev() {
                            let mut other_key = key.clone();
                            other_key.clip(l + 1);
                            other_key.flip(l);
                            new_node = Self::new_inner_unknown_order(
                                l,
                                new_node,
                                Self::new_leaf(other_key, old_value.clone()),
                            );
                        }
                        new_node
                    };
                    *self = new_node;
                    // we checked value before, nothing to compress
                    return;
                }
                Node::InnerNode(ref mut inner) => {
                    inner.pick_side_mut(&key).insert(key, value);
                }
            }
            // after recursion check for compression
            self.compress();
        }
    }

    fn compress(&mut self)
    where
        V: PartialEq,
    {
        let self_key_len = self.key().len();

        // compress: if node has two children, and both sub keys are
        // exactly one bit longer than the key of the parent node, and
        // both child nodes are leafs and share the same value, make the
        // current node a leaf
        let compress = match *self {
            Node::InnerNode(ref inner) => {
                let left_value = match inner.children.left {
                    Node::Leaf(ref leaf) if leaf.key.len() == self_key_len + 1 => &leaf.value,
                    _ => return, // not a leaf or more than one bit longer
                };
                let right_value = match inner.children.right {
                    Node::Leaf(ref leaf) if leaf.key.len() == self_key_len + 1 => &leaf.value,
                    _ => return, // not a leaf or more than one bit longer
                };
                left_value == right_value
            }
            Node::Leaf(_) => return, // already compressed
        };
        if compress {
            *self = match self {
                // move value from left
                Node::InnerNode(inner) => match &inner.children.left {
                    Node::Leaf(leaf) => Node::Leaf(Leaf {
                        key: inner.key.clone(),
                        value: leaf.value.clone(),
                    }),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
        }
    }
}

impl<V> Default for CidrMap<V>
where
    V: Clone,
{
    fn default() -> Self {
        Self { root: None }
    }
}

impl<V> CidrMap<V>
where
    V: Clone,
{
    pub fn new() -> Self {
        Self { root: None }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        self.get_prefix_match(ip).is_some()
    }

    pub fn get_prefix_match(&self, ip: IpAddr) -> Option<&V> {
        let key: AnyIpCidr = IpCidr::new_host(ip).into();
        self.get_prefix_match_cidr(&key)
    }

    pub fn get_prefix_match_cidr(&self, key: &AnyIpCidr) -> Option<&V> {
        let node = self.root.as_ref()?;
        Self::find_item(node, &key)
    }

    fn find_item<'a>(node: &'a Node<V>, ip: &AnyIpCidr) -> Option<&'a V> {
        match node {
            Node::Leaf(leaf) => {
                if leaf.key.contains(&ip.first_address().unwrap()) {
                    Some(&leaf.value)
                } else {
                    None
                }
            }
            Node::InnerNode(inner) => Self::find_item(inner.pick_side(&ip), ip),
        }
    }

    /// Add a new prefix => value mapping.
    ///
    /// As values can't be compared for equality it cannot merge
    /// neighbour prefixes that map to the same value.
    pub fn insert_uncompressed(&mut self, key: AnyIpCidr, value: V)
    where
        V: Clone,
    {
        match self.root {
            None => {
                self.root = Some(Node::new_leaf(key, value));
            }
            Some(ref mut node) => {
                node.insert_uncompressed(key, value);
            }
        }
    }

    /// Add a new prefix => value mapping.  (Partially) overwrites old
    /// mappings.
    pub fn insert(&mut self, key: AnyIpCidr, value: V)
    where
        V: Clone + PartialEq,
    {
        match self.root {
            None => {
                self.root = Some(Node::new_leaf(key, value));
            }
            Some(ref mut node) => {
                node.insert(key, value);
            }
        }
    }

    /// Read-only access to the tree.
    ///
    /// An empty map doesn't have any nodes (i.e. `None`).
    pub fn root(&self) -> Option<&Node<V>> {
        self.root.as_ref()
    }

    /// Iterate over all values in the map
    pub fn iter(&self) -> Iter<V> {
        Iter::new(self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Left,
    Right,
    Up,
}

/// Iterate over tree
pub struct Iter<'a, V: 'a>
where
    V: Clone,
{
    stack: Vec<(Direction, &'a Node<V>)>,
}

impl<'a, V> Iter<'a, V>
where
    V: Clone,
{
    /// new iterator
    pub fn new(tree: &'a CidrMap<V>) -> Self {
        match tree.root() {
            None => Iter { stack: Vec::new() },
            Some(node) => Iter {
                stack: vec![(Direction::Left, node)],
            },
        }
    }
}

impl<'a, V> Iterator for Iter<'a, V>
where
    V: Clone,
{
    type Item = (&'a AnyIpCidr, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        if self.stack.is_empty() {
            return None;
        }

        // go up in tree from last visited node
        while Direction::Up == self.stack[self.stack.len() - 1].0 {
            if 1 == self.stack.len() {
                self.stack.clear();
                return None;
            }

            self.stack.pop();
            // stack cannot be empty yet!
            debug_assert!(!self.stack.is_empty());
        }

        loop {
            let top = self.stack.len() - 1;
            let (dir, node) = self.stack[top];

            debug_assert!(!self.stack.is_empty());
            // go down in tree to next node
            match dir {
                Direction::Left => match *node {
                    Node::InnerNode(ref inner) => {
                        self.stack[top].0 = Direction::Right;
                        self.stack.push((Direction::Left, inner.left()));
                    }
                    Node::Leaf(ref leaf) => {
                        self.stack[top].0 = Direction::Up;
                        return Some((&leaf.key, &leaf.value));
                    }
                },
                Direction::Right => match *node {
                    Node::InnerNode(ref inner) => {
                        self.stack[top].0 = Direction::Up;
                        self.stack.push((Direction::Left, inner.right()));
                    }
                    Node::Leaf(_) => unreachable!(),
                },
                Direction::Up => unreachable!(),
            }
        }
    }
}

impl<S, V: Clone + PartialEq> FromIterator<(S, V)> for CidrMap<V>
where
    S: Into<AnyIpCidr>,
{
    fn from_iter<I: IntoIterator<Item = (S, V)>>(iter: I) -> Self {
        let mut map = CidrMap::new();
        for (key, value) in iter {
            map.insert(key.into(), value);
        }
        map
    }
}

impl<T: Ord + Into<AnyIpCidr>, const N: usize, V: Clone + Ord> From<[(T, V); N]> for CidrMap<V> {
    /// Converts a `[(T,V); N]` into a `CidrSet`.
    fn from(mut arr: [(T, V); N]) -> Self {
        if N == 0 {
            return CidrMap::new();
        }

        // use stable sort to preserve the insertion order.
        arr.sort();
        let iter = IntoIterator::into_iter(arr).map(|k| k);
        iter.collect()
    }
}

impl<V: Clone> Into<Vec<(AnyIpCidr, V)>> for CidrMap<V> {
    fn into(self) -> Vec<(AnyIpCidr, V)> {
        let mut result = vec![];
        for (key, value) in self.iter() {
            result.push((key.clone(), value.clone()));
        }
        result
    }
}

#[cfg(feature = "lua")]
impl LuaUserData for CidrMap<CacheValue> {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        mod_memoize::Memoized::impl_memoize(methods);
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            let key = parse_cidr_from_ip_and_or_port(&key).map_err(any_err)?;
            if let Some(value) = this.get_prefix_match_cidr(&key) {
                let value = value.as_lua(lua)?;
                Ok(Some(value))
            } else {
                Ok(None)
            }
        });
        methods.add_meta_method_mut(
            MetaMethod::NewIndex,
            |lua, this, (key, value): (String, mlua::Value)| {
                let key = parse_cidr_from_ip_and_or_port(&key).map_err(any_err)?;
                let value = CacheValue::from_lua(value, lua)?;
                this.insert(key, value);
                Ok(())
            },
        );
    }
}

#[cfg(feature = "lua")]
fn parse_cidr_from_ip_and_or_port(s: &str) -> anyhow::Result<AnyIpCidr> {
    match parse_cidr(s) {
        Ok(c) => Ok(c),
        Err(err) => {
            if s.starts_with('[') {
                if let Some((ip, _port)) = s[1..].split_once(']') {
                    return parse_cidr(ip).map_err(|err| {
                        anyhow::anyhow!(
                            "failed to parse '{ip}', the \
                             []-enclosed portion of '{s}', as an IP address: {err:#}"
                        )
                    });
                }
            }
            if let Some((ip, _port)) = s.rsplit_once(':') {
                return parse_cidr(ip).map_err(|err| {
                    anyhow::anyhow!(
                        "failed to parse '{ip}', the \
                         :-delimited portion of '{s}', as an IP address: {err:#}"
                    )
                });
            }
            Err(err)
        }
    }
}

/// The underlying AnyIpCidr::from_str parser is very strict and its error messages
/// are a little too terse.
/// We use this alternative parser to augment the error messages with more context
/// and suggestions.
// <https://github.com/stbuehler/rust-cidr/issues/8>
pub fn parse_cidr(s: &str) -> anyhow::Result<AnyIpCidr> {
    AnyIpCidr::from_str(s).map_err(|err| {
        match cidr::parsers::parse_any_cidr_full_ignore_hostbits(
            s,
            std::str::FromStr::from_str,
            std::str::FromStr::from_str,
        ) {
            Ok(loose) => {
                anyhow::anyhow!("{s} is not a valid CIDR: {err:#}. Did you mean {loose}?")
            }
            Err(err) => {
                anyhow::anyhow!("{s} is not a valid CIDR: {err:#}")
            }
        }
    })
}

#[cfg(feature = "lua")]
pub fn register(lua: &Lua) -> anyhow::Result<()> {
    use std::collections::HashMap;
    let cidr_mod = get_or_create_sub_module(lua, "cidr")?;

    cidr_mod.set(
        "make_map",
        lua.create_function(|lua, value: Option<HashMap<String, mlua::Value>>| {
            let mut cmap: CidrMap<mod_memoize::CacheValue> = CidrMap::new();

            if let Some(value) = value {
                for (k, v) in value {
                    let k = parse_cidr_from_ip_and_or_port(&k).map_err(any_err)?;
                    let v = CacheValue::from_lua(v, lua)?;
                    cmap.insert(k, v);
                }
            }

            Ok(cmap)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_error_message() {
        assert_eq!(
            parse_cidr("10.0.0.1/24").unwrap_err().to_string(),
            "10.0.0.1/24 is not a valid CIDR: host part of address was not zero. Did you mean 10.0.0.0/24?"
        );
    }

    #[test]
    fn cidrmap() {
        let set: CidrMap<&str> = [
            (parse_cidr("127.0.0.1").unwrap(), "loopbackv4"),
            (parse_cidr("::1").unwrap(), "loopbackv6"),
            (parse_cidr("192.168.1.0/24").unwrap(), ".1"),
            // This entry is overlapped by the preceding entry
            (parse_cidr("192.168.1.24").unwrap(), ".1"),
            (parse_cidr("192.168.3.0/28").unwrap(), ".3"),
            (parse_cidr("192.168.3.2").unwrap(), ".3.split"),
            (parse_cidr("10.0.3.0/24").unwrap(), "10.3"),
            (parse_cidr("10.0.4.0/24").unwrap(), "10.4"),
            (parse_cidr("10.0.7.0/24").unwrap(), "10.7"),
        ]
        .into();

        fn get<'a>(set: &'a CidrMap<&str>, key: &str) -> Option<&'a str> {
            let key = key.parse().unwrap();
            set.get_prefix_match(key).copied()
        }

        assert_eq!(get(&set, "127.0.0.1"), Some("loopbackv4"));
        assert_eq!(get(&set, "127.0.0.2"), None);
        assert_eq!(get(&set, "::1"), Some("loopbackv6"));

        assert_eq!(get(&set, "192.168.2.1"), None);

        assert_eq!(get(&set, "192.168.1.0"), Some(".1"));
        assert_eq!(get(&set, "192.168.1.1"), Some(".1"));
        assert_eq!(get(&set, "192.168.1.100"), Some(".1"));
        assert_eq!(get(&set, "192.168.1.24"), Some(".1"));

        assert_eq!(get(&set, "192.168.3.0"), Some(".3"));
        assert_eq!(get(&set, "192.168.3.16"), None);
        assert_eq!(get(&set, "192.168.3.2"), Some(".3.split"));

        // Note that the snapshot does not contain 192.168.1.24/32; that
        // overlaps with the broader 192.168.1.0/24 so is "lost"
        // when extracting the information from the set.
        // Furthermore, the .3.split value inserted for .3.2
        // causes more .3 entries to be generated to accomodate the
        // split in that subnet.
        k9::snapshot!(
            &set,
            r#"
{
    "10.0.3.0/24": "10.3",
    "10.0.4.0/24": "10.4",
    "10.0.7.0/24": "10.7",
    "127.0.0.1": "loopbackv4",
    "192.168.1.0/24": ".1",
    "192.168.3.0/31": ".3",
    "192.168.3.2": ".3.split",
    "192.168.3.3": ".3",
    "192.168.3.4/30": ".3",
    "192.168.3.8/29": ".3",
    "::1": "loopbackv6",
}
"#
        );
    }
}
