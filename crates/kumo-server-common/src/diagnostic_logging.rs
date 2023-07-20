use anyhow::Context;
use clap::ValueEnum;
use metrics_prometheus::recorder::Layer as _;
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer};

// Why in the heck is this a function and not simply the reload handle itself?
// The reason is because the tracing_subscriber crate makes heavy use of composed
// generic types and with the configuration we have chosen, some of the layers have
// `impl Layer` types that cannot be named here.
// <https://en.wiktionary.org/wiki/Voldemort_type>
//
// Even if it could be named, writing out its type here would make your eyes bleed.
// Even the rust compiler doesn't want to print the name, instead writing it out
// to a separate debugging file in its diagnostics!
//
// The approach taken is to stash a closure into this, and the closure capture
// the reload handle and operates upon it.
//
// This way we don't need to name the type, and won't need to struggle with re-naming
// it if we change the layering of the log subscriber.
static TRACING_FILTER_RELOAD_HANDLE: OnceCell<
    Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>,
> = OnceCell::new();

pub fn set_diagnostic_log_filter(new_filter: &str) -> anyhow::Result<()> {
    let func = TRACING_FILTER_RELOAD_HANDLE
        .get()
        .ok_or_else(|| anyhow::anyhow!("unable to retrieve filter reload handle"))?;
    (func)(new_filter)
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "kebab_case")]
pub enum DiagnosticFormat {
    Pretty,
    Full,
    Compact,
    Json,
}

pub struct LoggingConfig<'a> {
    pub tokio_console: bool,
    pub log_dir: Option<PathBuf>,
    pub filter_env_var: &'a str,
    pub default_filter: &'a str,
    pub diag_format: DiagnosticFormat,
}

impl<'a> LoggingConfig<'a> {
    pub fn init(&self) -> anyhow::Result<()> {
        let (non_blocking, _non_blocking_flusher);
        let log_writer = if let Some(log_dir) = &self.log_dir {
            let file_appender = tracing_appender::rolling::hourly(log_dir, "log");
            (non_blocking, _non_blocking_flusher) = tracing_appender::non_blocking(file_appender);
            BoxMakeWriter::new(non_blocking)
        } else {
            BoxMakeWriter::new(std::io::stderr)
        };

        let layer = fmt::layer().with_thread_names(true).with_writer(log_writer);
        let layer = match self.diag_format {
            DiagnosticFormat::Pretty => layer.pretty().boxed(),
            DiagnosticFormat::Full => layer.boxed(),
            DiagnosticFormat::Compact => layer.compact().boxed(),
            DiagnosticFormat::Json => layer.json().boxed(),
        };

        let env_filter = EnvFilter::try_new(
            std::env::var(self.filter_env_var)
                .as_deref()
                .unwrap_or(self.default_filter),
        )?;
        let (env_filter, reload_handle) = tracing_subscriber::reload::Layer::new(env_filter);
        tracing_subscriber::registry()
            .with(if self.tokio_console {
                Some(console_subscriber::spawn())
            } else {
                None
            })
            .with(layer.with_filter(env_filter))
            .with(metrics_tracing_context::MetricsLayer::new())
            .init();

        TRACING_FILTER_RELOAD_HANDLE
            .set(Box::new(move |new_filter: &str| {
                let f = EnvFilter::try_new(new_filter)
                    .with_context(|| format!("parsing log filter '{new_filter}'"))?;
                Ok(reload_handle.reload(f).context("applying new log filter")?)
            }))
            .map_err(|_| anyhow::anyhow!("failed to assign reloadable logging filter"))?;

        metrics::set_boxed_recorder(Box::new(
            metrics_tracing_context::TracingContextLayer::all()
                .layer(metrics_prometheus::Recorder::builder().build()),
        ))?;
        Ok(())
    }
}
