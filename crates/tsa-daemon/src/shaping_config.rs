use anyhow::Context;
use arc_swap::ArcSwap;
use config::CallbackSignature;
use kumo_api_types::shaping::Shaping;
use kumo_server_runtime::spawn;
use std::sync::{Arc, LazyLock};

static SHAPING: LazyLock<ArcSwap<Shaping>> =
    LazyLock::new(|| ArcSwap::from_pointee(Shaping::default()));

pub async fn load_shaping() -> anyhow::Result<Arc<Shaping>> {
    let mut config = config::load_config().await?;
    let sig = CallbackSignature::<(), Shaping>::new("tsa_load_shaping_data");
    let shaping: Shaping = config
        .async_call_callback_non_default(&sig, ())
        .await
        .context("in tsa_load_shaping_data event")?;
    config.put();
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
    spawn("shaping-updater", run_updater())?;
    Ok(())
}
