#![cfg(test)]
use std::sync::Once;
use tracing_subscriber::{fmt, EnvFilter};

static INIT: Once = Once::new();

/// Installs a tracing subscriber for the test process, routed through the
/// libtest capture so that events from in-process code appear with a failing
/// test's output. The daemons log separately to their own captured stderr.
///
/// Honors `RUST_LOG`, defaulting to `info`. Safe to call more than once.
pub fn init_test_logging() {
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init()
            .ok();
    });
}
