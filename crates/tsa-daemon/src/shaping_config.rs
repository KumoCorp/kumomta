use anyhow::Context;
use arc_swap::ArcSwap;
use config::CallbackSignature;
use kumo_api_types::shaping::Shaping;
use std::sync::{Arc, LazyLock};
use tokio::task::LocalSet;

static SHAPING: LazyLock<ArcSwap<Shaping>> =
    LazyLock::new(|| ArcSwap::from_pointee(Shaping::default()));

pub async fn load_shaping() -> anyhow::Result<Arc<Shaping>> {
    let mut config = config::load_config().await?;
    let sig = CallbackSignature::<(), Shaping>::new("tsa_load_shaping_data");
    let shaping: Shaping = config
        .async_call_callback_non_default(&sig, ())
        .await
        .context("in tsa_load_shaping_data event")?;
    Ok(Arc::new(shaping))
}

pub fn get_shaping() -> Arc<Shaping> {
    SHAPING.load_full()
}

pub fn assign_shaping(shaping: Arc<Shaping>) {
    SHAPING.store(shaping);
}

async fn run_updater() {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        match load_shaping().await {
            Ok(shaping) => {
                SHAPING.store(shaping);
            }
            Err(err) => {
                tracing::error!("{err:#}");
            }
        }
    }
}

pub fn spawn_shaping_updater() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name(format!("shaping-updater"))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .on_thread_park(|| kumo_server_memory::purge_thread_cache())
                .build()
                .unwrap();
            let local_set = LocalSet::new();
            local_set.block_on(&runtime, run_updater());
        })?;
    Ok(())
}
