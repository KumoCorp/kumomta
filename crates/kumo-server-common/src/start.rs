use crate::diagnostic_logging::LoggingConfig;
use anyhow::Context;
use config::RegisterFunc;
use kumo_server_lifecycle::LifeCycle;
use kumo_server_runtime::rt_spawn;
use std::future::Future;
use std::path::Path;
pub struct StartConfig<'a> {
    pub logging: LoggingConfig<'a>,
    pub lua_funcs: &'a [RegisterFunc],
    pub policy: &'a Path,
}

impl<'a> StartConfig<'a> {
    pub async fn run<INIT, FINI>(
        self,
        init_future: INIT,
        shutdown_future: FINI,
    ) -> anyhow::Result<()>
    where
        INIT: Future<Output = anyhow::Result<()>> + Send + 'static,
        FINI: Future<Output = ()> + Send + 'static,
    {
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

        let mut life_cycle = LifeCycle::new();

        let init_handle = rt_spawn("initialize".to_string(), async move {
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
