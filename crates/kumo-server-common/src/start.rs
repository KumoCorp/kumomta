use crate::diagnostic_logging::LoggingConfig;
use crate::nodeid::NodeId;
use anyhow::Context;
use chrono::{DateTime, Utc};
use config::RegisterFunc;
use kumo_machine_info::MachineInfo;
use kumo_prometheus::declare_metric;
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

        start_cpu_usage_monitor();

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

declare_metric! {
/// The sum of the system-wide CPU usage for each CPU in the system, can add up to more than 100%.
///
/// Each CPU has a value from 0-100% busy; a value of 100% in this metric
/// indicates that the load is equivalent to one fully utilized CPU.
///
/// A multi-CPU system can report more than 100% in this metric; a dual-CPU
/// system reporting 200% indicates that both CPUs are fully utilized.
///
/// See system_cpu_usage_normalized for a version of this metric that scales from
/// 0% (totally idle) to 100% (totally saturated).
///
/// This metric is scoped to the system, reflecting the total load on the
/// system, not just from the kumo related process(es).
static SYS_CPU_USAGE: IntGauge("system_cpu_usage_sum");
}

declare_metric! {
/// The sum of the system-wide CPU usage for each CPU in the system, divided by the number of CPUs.
///
/// 100% in this metric indicates that all CPU cores are 100% busy.
///
/// This metric is scoped to the system, reflecting the total load on the
/// system, not just from the kumo related process(es).
static SYS_CPU_USAGE_NORM: IntGauge("system_cpu_usage_normalized");
}

declare_metric! {
/// The sum of the process CPU usage for each CPU in the system, can add up to more than 100%.
///
/// Each CPU has a value from 0-100% busy; a value of 100% in this metric
/// indicates that the load is equivalent to one fully utilized CPU.
///
/// A multi-CPU system can report more than 100% in this metric; a dual-CPU
/// system reporting 200% indicates that both CPUs are fully utilized.
///
/// See process_cpu_usage_normalized for a version of this metric that scales from
/// 0% (totally idle) to 100% (totally saturated).
///
/// This metric is scoped to the service process, reflecting the CPU used only
/// by the process and not the system as a whole.
static PROC_CPU_USAGE: IntGauge("process_cpu_usage_sum");
}

declare_metric! {
/// The sum of the process CPU usage for each CPU in the system, divided by the number of CPUs.
///
/// 100% in this metric indicates that all CPU cores are 100% busy.
///
/// This metric is scoped to the service process, reflecting the CPU used only
/// by the process and not the system as a whole.
static PROC_CPU_USAGE_NORM: IntGauge("process_cpu_usage_normalized");
}

fn start_cpu_usage_monitor() {
    std::thread::spawn(|| {
        use std::time::Duration;
        use sysinfo::{
            get_current_pid, CpuRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate,
            RefreshKind, System,
        };

        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        );

        let my_pid = get_current_pid().expect("failed to get own pid!?");

        let update_interval = Duration::from_secs(3).max(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        // Do two initial updates so that we have sufficient data for
        // computing a delta; one here outside the loop, and another
        // at the top of the loop.
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

        loop {
            sys.refresh_cpu_usage();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[my_pid]),
                true,
                ProcessRefreshKind::everything().without_tasks(),
            );

            let num_cpus = sys.cpus().len() as i64;

            let sys_usage = sys.cpus().iter().map(|cpu| cpu.cpu_usage()).sum::<f32>() as i64;
            SYS_CPU_USAGE.set(sys_usage);
            SYS_CPU_USAGE_NORM.set(sys_usage / num_cpus);

            if let Some(p) = sys.process(Pid::from(my_pid)) {
                let proc_usage = p.cpu_usage() as i64;
                PROC_CPU_USAGE.set(proc_usage);
                PROC_CPU_USAGE_NORM.set(proc_usage / num_cpus);
            }

            std::thread::sleep(update_interval);
        }
    });
}
