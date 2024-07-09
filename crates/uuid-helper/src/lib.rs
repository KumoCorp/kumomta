use once_cell::sync::Lazy;
pub use uuid;
use uuid::Uuid;

static MAC: Lazy<[u8; 6]> = Lazy::new(get_mac_address_once);

/// Obtain the mac address of the first non-loopback interface on the system.
/// If there are no candidate interfaces, fall back to the `gethostid()` function,
/// which will attempt to load a host id from a file on the filesystem, or if that
/// fails, resolve the hostname of the node to its IPv4 address using a reverse DNS
/// lookup, and then derive some 32-bit number from that address through unspecified
/// means.
fn get_mac_address_once() -> [u8; 6] {
    match mac_address::get_mac_address() {
        Ok(Some(addr)) => addr.bytes(),
        _ => {
            // Fall back to gethostid, which is not great, but
            // likely better than just random numbers
            let host_id = unsafe { libc::gethostid() }.to_le_bytes();
            let mac: [u8; 6] = [
                host_id[0], host_id[1], host_id[2], host_id[3], host_id[4], host_id[5],
            ];
            mac
        }
    }
}

pub fn get_mac_address() -> &'static [u8; 6] {
    &*MAC
}

pub fn now_v1() -> uuid::Uuid {
    Uuid::now_v1(&*MAC)
}

pub fn new_v1(ts: uuid::timestamp::Timestamp) -> uuid::Uuid {
    Uuid::new_v1(ts, &*MAC)
}
