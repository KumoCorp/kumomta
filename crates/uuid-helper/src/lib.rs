pub use kumo_mac_address::get_mac_address;
pub use uuid;
use uuid::Uuid;

pub fn now_v1() -> uuid::Uuid {
    Uuid::now_v1(get_mac_address())
}

pub fn new_v1(ts: uuid::timestamp::Timestamp) -> uuid::Uuid {
    Uuid::new_v1(ts, get_mac_address())
}
