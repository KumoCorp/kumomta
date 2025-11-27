use anyhow::Context;
use clap::Parser;
use config::CallbackSignature;
use data_loader::KeySource;
use kumo_server_common::diagnostic_logging::{DiagnosticFormat, LoggingConfig};
use kumo_server_common::start::StartConfig;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use std::path::PathBuf;
use std::time::Duration;

mod mod_proxy;
mod proxy_handler;

/// KumoProxy SOCKS5 Proxy Server.
///
/// Full docs available at: <https://docs.kumomta.com>
#[derive(Debug, Parser)]
#[command(about, version=version_info::kumo_version())]
struct Opt {
    /// Lua policy file to load.
    /// Mutually exclusive with --listen and other legacy options.
    #[arg(long)]
    policy: Option<PathBuf>,

    /// Directory where diagnostic log files will be placed.
    ///
    /// If omitted, diagnostics will be printed to stderr.
    #[arg(long)]
    diag_log_dir: Option<PathBuf>,

    /// How diagnostic logs render. full, compact and pretty are intended
    /// for human consumption.
    ///
    /// json outputs machine readable records.
    #[arg(long, default_value = "full")]
    diag_format: DiagnosticFormat,

    // Legacy CLI options for backwards compatibility
    // These are mutually exclusive with --policy
    /// [Legacy] Address(es) to listen on (e.g., "0.0.0.0:5000").
    /// Mutually exclusive with --policy.
    #[arg(long)]
    listen: Vec<String>,

    /// [Legacy] Disable zero-copy splice optimization on Linux.
    #[arg(long)]
    no_splice: bool,

    /// [Legacy] Timeout in seconds for all I/O operations.
    #[arg(long, default_value = "60")]
    timeout_seconds: u64,

    /// [Legacy] Path to TLS certificate file (PEM format).
    #[arg(long)]
    tls_certificate: Option<PathBuf>,

    /// [Legacy] Path to TLS private key file (PEM format).
    #[arg(long)]
    tls_private_key: Option<PathBuf>,
}

impl Opt {
    /// Check if legacy CLI mode is being used
    fn is_legacy_mode(&self) -> bool {
        !self.listen.is_empty() || self.tls_certificate.is_some() || self.tls_private_key.is_some()
    }

    /// Validate CLI options
    fn validate(&self) -> anyhow::Result<()> {
        if self.policy.is_some() && self.is_legacy_mode() {
            anyhow::bail!(
                "--policy cannot be used with legacy options (--listen, --tls-certificate, etc.). \
                 Use either --policy for Lua configuration or legacy CLI options, but not both."
            );
        }

        if self.is_legacy_mode() {
            if self.listen.is_empty() {
                anyhow::bail!(
                    "No listeners defined! Use --listen to specify at least one, \
                     or use --policy for Lua-based configuration."
                );
            }

            if self.tls_private_key.is_some() != self.tls_certificate.is_some() {
                anyhow::bail!(
                    "--tls-certificate and --tls-private-key must be specified together, or not at all"
                );
            }
        }

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();
    opts.validate()?;

    let (_no_file_soft, no_file_hard) = getrlimit(Resource::RLIMIT_NOFILE)?;
    setrlimit(Resource::RLIMIT_NOFILE, no_file_hard, no_file_hard).context("setrlimit NOFILE")?;

    kumo_server_common::panic::register_panic_hook();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_park(kumo_server_memory::purge_thread_cache)
        .build()
        .unwrap()
        .block_on(async move { run(opts).await })
}

async fn perform_init() -> anyhow::Result<()> {
    tracing::info!("Version is {}", version_info::kumo_version());
    let mut config = config::load_config().await?;

    let proxy_init_sig = CallbackSignature::<(), ()>::new("proxy_init");

    config
        .async_call_callback(&proxy_init_sig, ())
        .await
        .context("in proxy_init event")?;
    config.put();

    Ok(())
}

async fn signal_shutdown() {
    tracing::info!("shutting down");
}

async fn run(opts: Opt) -> anyhow::Result<()> {
    kumo_server_runtime::assign_main_runtime(tokio::runtime::Handle::current());

    if opts.is_legacy_mode() {
        // Legacy mode: use CLI options directly without Lua
        run_legacy(opts).await
    } else {
        // Modern mode: use Lua policy file
        let policy = opts
            .policy
            .unwrap_or_else(|| PathBuf::from("/opt/kumomta/etc/proxy/init.lua"));

        StartConfig {
            logging: LoggingConfig {
                log_dir: opts.diag_log_dir.clone(),
                diag_format: opts.diag_format,
                filter_env_var: "KUMO_PROXY_LOG",
                default_filter:
                    "proxy_server=info,kumo_server_common=info,kumo_server_runtime=info",
            },
            lua_funcs: &[kumo_server_common::register, mod_proxy::register],
            policy: &policy,
        }
        .run(perform_init(), signal_shutdown())
        .await
    }
}

/// Run in legacy mode using CLI options (backwards compatibility)
async fn run_legacy(opts: Opt) -> anyhow::Result<()> {
    use kumo_server_common::diagnostic_logging::LoggingConfig;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    // Initialize logging
    LoggingConfig {
        log_dir: opts.diag_log_dir.clone(),
        diag_format: opts.diag_format,
        filter_env_var: "KUMO_PROXY_LOG",
        default_filter: "proxy_server=info,kumo_server_common=info,kumo_server_runtime=info",
    }
    .init()
    .context("failed to initialize logging")?;

    tracing::info!("Version is {} (legacy mode)", version_info::kumo_version());

    let timeout = Duration::from_secs(opts.timeout_seconds);

    // Build TLS acceptor if TLS is configured
    let tls_acceptor =
        if let (Some(cert_path), Some(key_path)) = (&opts.tls_certificate, &opts.tls_private_key) {
            let hostname = gethostname::gethostname()
                .to_str()
                .unwrap_or("localhost")
                .to_string();

            let cert_source = KeySource::File(cert_path.display().to_string());
            let key_source = KeySource::File(key_path.display().to_string());

            let config = kumo_server_common::tls_helpers::make_server_config(
                &hostname,
                &Some(key_source),
                &Some(cert_source),
                &None,
            )
            .await
            .context("failed to create TLS configuration")?;

            tracing::info!("TLS enabled");
            Some(TlsAcceptor::from(config))
        } else {
            None
        };

    // Start listeners
    for endpoint in &opts.listen {
        let listener = TcpListener::bind(endpoint)
            .await
            .with_context(|| format!("failed to bind to {endpoint}"))?;

        let local_address = listener.local_addr()?;
        let tls_acceptor = tls_acceptor.clone();
        let no_splice = opts.no_splice;

        if tls_acceptor.is_some() {
            tracing::info!("proxy listener (TLS) on {local_address:?}");
        } else {
            tracing::info!("proxy listener on {local_address:?}");
        }

        tokio::spawn(async move {
            legacy_accept_loop(listener, local_address, timeout, no_splice, tls_acceptor).await;
        });
    }

    tracing::info!("initialization complete");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}

async fn legacy_accept_loop(
    listener: tokio::net::TcpListener,
    local_address: std::net::SocketAddr,
    timeout: Duration,
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))] no_splice: bool,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
) {
    loop {
        let (socket, peer_address) = match listener.accept().await {
            Ok(tuple) => tuple,
            Err(err) => {
                tracing::error!("accept failed: {err:#}");
                return;
            }
        };

        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            let result = if let Some(acceptor) = tls_acceptor {
                match acceptor.accept(socket).await {
                    Ok(tls_stream) => {
                        proxy_handler::handle_proxy_client(
                            tls_stream,
                            peer_address,
                            local_address,
                            timeout,
                            no_splice,
                            false, // No auth in legacy mode without Lua
                        )
                        .await
                    }
                    Err(err) => {
                        tracing::debug!("TLS handshake failed from {peer_address:?}: {err:#}");
                        return;
                    }
                }
            } else {
                proxy_handler::handle_proxy_client(
                    socket,
                    peer_address,
                    local_address,
                    timeout,
                    no_splice,
                    false, // No auth in legacy mode without Lua
                )
                .await
            };

            if let Err(err) = result {
                tracing::error!("proxy session error from {peer_address:?}: {err:#}");
            }
        });
    }
}
