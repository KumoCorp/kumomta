use crate::diagnostic_logging::LoggingConfig;
use crate::nodeid::NodeId;
use anyhow::Context;
use chrono::{DateTime, Utc};
use config::RegisterFunc;
use kumo_machine_info::MachineInfo;
use kumo_server_lifecycle::LifeCycle;
use kumo_server_runtime::rt_spawn;
use parking_lot::Mutex;
use std::future::Future;
use std::path::Path;
use std::sync::LazyLock;

pub static ONLINE_SINCE: LazyLock<DateTime<Utc>> = LazyLock::new(Utc::now);
pub static MACHINE_INFO: LazyLock<Mutex<Option<MachineInfo>>> = LazyLock::new(|| Mutex::new(None));

pub struct StartConfig<'a> {
    pub logging: LoggingConfig<'a>,
    pub lua_funcs: &'a [RegisterFunc],
    pub policy: &'a Path,
}

impl StartConfig<'_> {
    pub async fn run<INIT, FINI>(
        self,
        init_future: INIT,
        shutdown_future: FINI,
    ) -> anyhow::Result<()>
    where
        INIT: Future<Output = anyhow::Result<()>> + Send + 'static,
        FINI: Future<Output = ()> + Send + 'static,
    {
        LazyLock::force(&ONLINE_SINCE);
        self.logging.init()?;

        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| anyhow::anyhow!("failed to install default crypto provider"))?;

        kumo_server_memory::setup_memory_limit().context("setup_memory_limit")?;

        prometheus::register(Box::new(
            tokio_metrics_collector::default_runtime_collector(),
        ))
        .context("failed to configure tokio-metrics-collector")?;

        for &func in self.lua_funcs {
            config::register(func);
        }

        config::set_policy_path(self.policy.to_path_buf())
            .await
            .with_context(|| format!("Error evaluating policy file {:?}", self.policy))?;

        tokio::spawn(async move {
            let mut info = MachineInfo::new();
            info.node_id.replace(NodeId::get().uuid.to_string());
            info.query_cloud_provider().await;
            *MACHINE_INFO.lock() = Some(info);
        });

        let mut life_cycle = LifeCycle::new();

        let init_handle = rt_spawn("initialize", async move {
            let mut error = None;
            if let Err(err) = init_future.await {
                let err = format!("{err:#}");
                tracing::error!("problem initializing: {err}");
                LifeCycle::request_shutdown().await;
                error.replace(err);
            }
            // This log line is depended upon by the integration
            // test harness. Do not change or remove it without
            // making appropriate adjustments over there!
            tracing::info!("initialization complete");
            error
        })?;

        life_cycle.wait_for_shutdown().await;

        // after waiting for those to idle out, shut down logging
        shutdown_future.await;

        tracing::info!("Shutdown completed OK!");

        if let Some(error) = init_handle.await? {
            anyhow::bail!("Initialization raised an error: {error}");
        }
        Ok(())
    }
}
