//! Resolves the 6 byte MAC address that identifies this node.
//!
//! The MAC becomes part of the node identity embedded in v1 UUIDs (spool ids,
//! node ids) and is reported in machine info. It must be distinct between
//! hosts. The naive "first interface the OS lists" choice is unreliable: on
//! cloud and containerized hosts it frequently selects a virtual device (a
//! container bridge, a veth pair) whose address is identical across otherwise
//! separate machines, which then produces colliding node identities.
//!
//! To avoid that, resolution accepts explicit operator overrides and, failing
//! those, prefers a physical interface over a virtual one. One cached value is
//! shared by every consumer so the id embedded in spool ids and the address
//! shown in machine info always agree.

use std::sync::LazyLock;

use mac_address::mac_address_by_name;
use nix::ifaddrs::getifaddrs;

/// Environment variable naming a literal MAC address to use verbatim, in the
/// usual colon or hyphen separated hex form. Each host must set a distinct
/// value. A shared value recreates the cross-host collisions that this
/// resolution logic exists to prevent.
const ENV_MAC_ADDRESS: &str = "KUMO_MAC_ADDRESS";

/// Environment variable naming the network interface (such as `eth0` or `ens5`)
/// whose MAC address identifies this node.
const ENV_MAC_INTERFACE: &str = "KUMO_MAC_INTERFACE";

/// Interface names for virtual devices, matched exactly. The loopback device
/// is the only single-instance virtual device we skip by name, and matching it
/// by prefix would also catch real interfaces an operator named `lom0` or
/// similar.
const VIRTUAL_INTERFACE_NAMES: &[&str] = &["lo", "lo0"];

/// Interface name prefixes for virtual device families whose MAC is often
/// identical across otherwise distinct hosts. These come in numbered instances
/// (`docker0`, `veth1234`), and a prefix is the most reliable way to catch
/// every one. Automatic selection skips these in favor of a physical interface.
const VIRTUAL_INTERFACE_PREFIXES: &[&str] = &[
    "br-", "cali", "cni", "docker", "dummy", "flannel", "gre", "ifb", "ip6tnl", "kube", "nlmon",
    "sit", "tap", "tun", "veth", "virbr", "vmbr", "vnet", "wg", "zt",
];

static MAC: LazyLock<[u8; 6]> = LazyLock::new(resolve);

/// Returns the MAC address that identifies this node. The value is resolved
/// once on first use and cached for the lifetime of the process. Any
/// environment overrides must be set before this is first called.
pub fn get_mac_address() -> &'static [u8; 6] {
    &MAC
}

/// Work through the override, physical-interface, and gethostid sources in
/// preference order, returning the first that yields a usable address.
fn resolve() -> [u8; 6] {
    if let Some(mac) = from_env_literal() {
        tracing::info!("using node MAC {} from {ENV_MAC_ADDRESS}", format_mac(&mac));
        return mac;
    }

    if let Some((mac, name)) = from_env_interface() {
        tracing::info!(
            "using node MAC {} from interface {name} named by {ENV_MAC_INTERFACE}",
            format_mac(&mac)
        );
        return mac;
    }

    if let Some((mac, name)) = first_physical_interface() {
        tracing::info!("using node MAC {} from interface {name}", format_mac(&mac));
        return mac;
    }

    let mac = from_host_id();
    tracing::warn!(
        "no usable network interface MAC found; derived node MAC {} from gethostid()",
        format_mac(&mac)
    );
    mac
}

fn from_env_literal() -> Option<[u8; 6]> {
    let value = std::env::var(ENV_MAC_ADDRESS).ok()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    match parse_mac(value) {
        Some(mac) if mac.iter().all(|b| *b == 0) => {
            tracing::error!("{ENV_MAC_ADDRESS}=`{value}` is a zero MAC address; ignoring it");
            None
        }
        Some(mac) => Some(mac),
        None => {
            tracing::error!("{ENV_MAC_ADDRESS}=`{value}` is not a valid MAC address; ignoring it");
            None
        }
    }
}

fn from_env_interface() -> Option<([u8; 6], String)> {
    let name = std::env::var(ENV_MAC_INTERFACE).ok()?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    match mac_address_by_name(&name) {
        Ok(Some(addr)) => {
            let bytes = addr.bytes();
            if bytes.iter().all(|b| *b == 0) {
                tracing::error!(
                    "interface {name} from {ENV_MAC_INTERFACE} has a zero MAC address; ignoring it"
                );
                None
            } else {
                Some((bytes, name))
            }
        }
        Ok(None) => {
            tracing::error!("interface {name} from {ENV_MAC_INTERFACE} was not found; ignoring it");
            None
        }
        Err(err) => {
            tracing::error!(
                "failed to read MAC for interface {name} from {ENV_MAC_INTERFACE}: {err:#}; \
                 ignoring it"
            );
            None
        }
    }
}

/// Returns the first physical interface's MAC together with its name, skipping
/// zero addresses and virtual devices. When every candidate is virtual it
/// returns the first non-zero one anyway, since a stable virtual address is
/// still preferable to the gethostid fallback.
///
/// The name and address come from the same `getifaddrs` entry. A separate
/// by-MAC name lookup would misattribute the name when several interfaces
/// share a MAC (bonding, macvlan).
fn first_physical_interface() -> Option<([u8; 6], String)> {
    let mut first_usable = None;
    for iface in getifaddrs().ok()? {
        let Some(bytes) = iface.address.and_then(|a| a.as_link_addr()?.addr()) else {
            continue;
        };
        if bytes.iter().all(|b| *b == 0) {
            continue;
        }
        let name = iface.interface_name;
        if first_usable.is_none() {
            first_usable = Some((bytes, name.clone()));
        }
        if is_virtual_interface(&name) {
            continue;
        }
        return Some((bytes, name));
    }
    first_usable
}

fn is_virtual_interface(name: &str) -> bool {
    VIRTUAL_INTERFACE_NAMES.contains(&name)
        || VIRTUAL_INTERFACE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Derives a 6 byte value from `gethostid()` as a last resort when no interface
/// offers a usable MAC. This is not guaranteed unique between hosts, but is
/// preferable to random bytes because it is stable across restarts.
fn from_host_id() -> [u8; 6] {
    let host_id = unsafe { libc::gethostid() }.to_le_bytes();
    [
        host_id[0], host_id[1], host_id[2], host_id[3], host_id[4], host_id[5],
    ]
}

/// Parses a MAC address as six 2-digit hex groups separated consistently by
/// `:` or `-`, or as 12 bare hex digits. Mixed separators are rejected rather
/// than normalized, since a hand-typed value with an inconsistent separator is
/// more likely a typo than an intended address.
fn parse_mac(input: &str) -> Option<[u8; 6]> {
    let groups: Vec<&str> = if input.contains(':') {
        input.split(':').collect()
    } else if input.contains('-') {
        input.split('-').collect()
    } else {
        if input.len() != 12 {
            return None;
        }
        return parse_hex_bytes(input);
    };
    if groups.len() != 6 || groups.iter().any(|g| g.len() != 2) {
        return None;
    }
    parse_hex_bytes(&groups.concat())
}

fn parse_hex_bytes(hex: &str) -> Option<[u8; 6]> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 6];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn format_mac(mac: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_colon_separated() {
        assert_eq!(
            parse_mac("02:1a:2b:3c:4d:5e"),
            Some([0x02, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e])
        );
    }

    #[test]
    fn parse_hyphen_separated() {
        assert_eq!(
            parse_mac("02-1A-2B-3C-4D-5E"),
            Some([0x02, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e])
        );
    }

    #[test]
    fn parse_bare_hex() {
        assert_eq!(
            parse_mac("021a2b3c4d5e"),
            Some([0x02, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e])
        );
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert_eq!(parse_mac("nope"), None);
        assert_eq!(parse_mac("02:1a:2b:3c:4d"), None);
        assert_eq!(parse_mac("02:1a:2b:3c:4d:5e:6f"), None);
        assert_eq!(parse_mac("gg:1a:2b:3c:4d:5e"), None);
    }

    #[test]
    fn parse_rejects_mixed_separators() {
        assert_eq!(parse_mac("02:1a-2b:3c-4d:5e"), None);
        assert_eq!(parse_mac("021a2b-3c4d5e"), None);
    }

    #[test]
    fn virtual_interfaces_match() {
        assert!(is_virtual_interface("lo"));
        assert!(is_virtual_interface("lo0"));
        assert!(is_virtual_interface("docker0"));
        assert!(is_virtual_interface("veth1234"));
        assert!(is_virtual_interface("br-abcdef"));
        assert!(is_virtual_interface("dummy0"));
        assert!(is_virtual_interface("gre0"));
        assert!(is_virtual_interface("sit0"));
        assert!(!is_virtual_interface("lom0"));
        assert!(!is_virtual_interface("eth0"));
        assert!(!is_virtual_interface("ens5"));
        assert!(!is_virtual_interface("bond0"));
    }
}
