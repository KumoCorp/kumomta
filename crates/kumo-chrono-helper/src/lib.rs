pub use chrono;
pub use chrono::{DateTime, TimeZone, Utc};

// chrono has a slightly awkward API that returns Option<T> in
// a few cases; for small constant values it makes the call site
// more complex, especially because the internally panicking variants
// of the constructor methods are now deprecated, forcing the call
// site to translate to an error inline or otherwise expect()
// the error away.
// This little crate exposes a few constants for infallible cases
// and wrapper functions that explain the error condition for the others.

#[allow(deprecated)]
pub const MINUTE: chrono::Duration = chrono::Duration::minutes(1);

#[allow(deprecated)]
pub const SECOND: chrono::Duration = chrono::Duration::seconds(1);

#[allow(deprecated)]
pub const HOUR: chrono::Duration = chrono::Duration::hours(1);

pub fn seconds(seconds: i64) -> anyhow::Result<chrono::Duration> {
    chrono::Duration::try_seconds(seconds).ok_or_else(|| {
        anyhow::anyhow!("{seconds} is out of range for chrono::Duration::try_seconds")
    })
}

pub fn minutes(minutes: i64) -> anyhow::Result<chrono::Duration> {
    chrono::Duration::try_minutes(minutes).ok_or_else(|| {
        anyhow::anyhow!("{minutes} is out of range for chrono::Duration::try_minutes")
    })
}

pub fn days(days: i64) -> anyhow::Result<chrono::Duration> {
    chrono::Duration::try_days(days)
        .ok_or_else(|| anyhow::anyhow!("{days} is out of range for chrono::Duration::try_days"))
}
