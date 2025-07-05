use anyhow::Context;
use clap::Parser;
use kumo_api_types::egress_path::parse_openssl_options;
use rfc5321::openssl::ssl::SslOptions;
use rfc5321::tokio_rustls::rustls::crypto::aws_lc_rs::ALL_CIPHER_SUITES;
use rfc5321::tokio_rustls::rustls::SupportedCipherSuite;
use rfc5321::{SmtpClient, SmtpClientTimeouts, TlsOptions};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// Show information about available TLS ciphers and capabilities
/// of a remote host.
#[derive(Clone, Debug, Parser)]
#[command(about = "MX TLS prober")]
struct Opt {
    #[command(subcommand)]
    cmd: SubCommand,
}

#[derive(Clone, Debug, Parser)]
enum SubCommand {
    /// Probe an MX host to see if it supports STARTTLS and
    /// information about its TLS support
    Probe(ProbeCommand),
    /// Show a list of all cipher suites supported by rustls
    ListRustlsCipherSuites,
}

#[derive(Clone, Debug, Parser)]
struct ProbeCommand {
    /// Disable SSL certificate verification; the connection
    /// will be private but you cannot trust that the peer
    /// is who they claim to be
    #[arg(long)]
    insecure: bool,
    #[arg(long)]
    prefer_openssl: bool,
    #[arg(long, value_parser=clap::builder::ValueParser::new(find_suite))]
    rustls_cipher_suites: Vec<SupportedCipherSuite>,
    #[arg(long)]
    certificate: Option<String>,
    #[arg(long)]
    private_key: Option<String>,
    #[arg(long)]
    openssl_cipher_list: Option<String>,
    #[arg(long)]
    openssl_cipher_suites: Option<String>,
    #[arg(long, value_parser=clap::builder::ValueParser::new(parse_openssl_options))]
    openssl_options: Option<SslOptions>,
    target: String,
}

fn find_suite(name: &str) -> anyhow::Result<SupportedCipherSuite> {
    kumo_api_types::egress_path::find_rustls_cipher_suite(name)
        .ok_or_else(|| anyhow::anyhow!("{name} is not a valid rustls cipher suite"))
}

async fn load_file(filename: Option<String>) -> anyhow::Result<Option<Arc<Box<[u8]>>>> {
    match &filename {
        Some(f) => {
            let mut file = File::open(f).await?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).await?;
            let boxed: Box<[u8]> = buffer.into_boxed_slice();
            Ok(Some(Arc::new(boxed)))
        }
        None => Ok(None),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .parse_filters("tls_probe=info,rustls=trace")
        .parse_env("RUST_LOG")
        .init();
    let opts = Opt::parse();

    match opts.cmd {
        SubCommand::ListRustlsCipherSuites => {
            for s in ALL_CIPHER_SUITES {
                println!("{s:?}");
            }
            Ok(())
        }
        SubCommand::Probe(probe) => {
            let timeouts = SmtpClientTimeouts::default();
            let mut client = SmtpClient::new(&probe.target, timeouts)
                .await
                .with_context(|| format!("failed to connect to {}", probe.target))?;

            let banner_timeout = timeouts.banner_timeout;
            let banner = client
                .read_response(None, banner_timeout)
                .await
                .with_context(|| format!("waiting for banner from {}", probe.target))?;
            anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");

            let caps = client.ehlo("there").await?;
            println!("{caps:#?}");

            let certificate_from_pem = load_file(probe.certificate).await?;
            let private_key_from_pem = load_file(probe.private_key).await?;

            if caps.contains_key("STARTTLS") {
                let tls_result = client
                    .starttls(TlsOptions {
                        insecure: probe.insecure,
                        prefer_openssl: probe.prefer_openssl,
                        alt_name: None,
                        dane_tlsa: vec![],
                        rustls_cipher_suites: probe.rustls_cipher_suites,
                        private_key_from_pem: private_key_from_pem,
                        certificate_from_pem: certificate_from_pem,
                        openssl_cipher_list: probe.openssl_cipher_list,
                        openssl_cipher_suites: probe.openssl_cipher_suites,
                        openssl_options: probe.openssl_options,
                    })
                    .await?;
                println!("{tls_result:?}");

                let caps = client.ehlo("there").await?;
                println!("EHLO after STARTTLS: {caps:#?}");
            }

            Ok(())
        }
    }
}
