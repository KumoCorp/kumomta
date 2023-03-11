pub fn kumo_version() -> &'static str {
    // See build.rs
    env!("KUMO_CI_TAG")
}

pub fn kumo_target_triple() -> &'static str {
    // See build.rs
    env!("KUMO_TARGET_TRIPLE")
}
