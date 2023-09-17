use crate::diagnostic_logging::LoggingConfig;
use anyhow::Context;
use config::RegisterFunc;
use kumo_server_lifecycle::LifeCycle;
use kumo_server_runtime::rt_spawn;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

pub struct StartConfig<'a> {
    pub logging: LoggingConfig<'a>,
    pub lua_funcs: &'a [RegisterFunc],
    pub policy: &'a Path,
}

impl<'a> StartConfig<'a> {
    pub async fn run<INIT, FINI>(
        self,
        perform_init: INIT,
        broadcast_shutdown: FINI,
    ) -> anyhow::Result<()>
    where
        INIT: FnOnce() -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> + Send + 'static,
        FINI: FnOnce() -> Pin<Box<dyn Future<Output = ()>>> + Send + 'static,
    {
        self.logging.init()?;

        kumo_server_memory::setup_memory_limit().context("setup_memory_limit")?;

        for &func in self.lua_funcs {
            config::register(func);
        }

        config::set_policy_path(self.policy.to_path_buf())
            .await
            .with_context(|| format!("set_policy_path to {:?}", self.policy))?;

        let mut life_cycle = LifeCycle::new();

        let init_handle = rt_spawn("initialize".to_string(), move || {
            Ok(async move {
                let mut error = None;
                let init_future = (perform_init)();
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
            })
        })
        .await?;

        life_cycle.wait_for_shutdown().await;

        // after waiting for those to idle out, shut down logging
        let shutdown_future = (broadcast_shutdown)();
        shutdown_future.await;

        tracing::info!("Shutdown completed OK!");

        if let Some(error) = init_handle.await? {
            anyhow::bail!("Initialization raised an error: {error}");
        }
        Ok(())
    }
}
