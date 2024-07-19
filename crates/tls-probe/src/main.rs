use clap::Parser;
use rfc5321::{SmtpClient, SmtpClientTimeouts, TlsOptions};

#[derive(Clone, Debug, Parser)]
#[command(about = "MX TLS prober")]
struct Opt {
    #[arg(long)]
    insecure: bool,
    #[arg(long)]
    prefer_openssl: bool,
    target: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let opts = Opt::parse();

    let timeouts = SmtpClientTimeouts::default();
    let mut client = SmtpClient::new(&opts.target, timeouts).await?;

    let banner_timeout = timeouts.banner_timeout;
    let banner = client.read_response(None, banner_timeout).await?;
    anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");

    let caps = client.ehlo("there").await?;
    println!("{caps:#?}");

    if caps.contains_key("STARTTLS") {
        let tls_result = client
            .starttls(TlsOptions {
                insecure: opts.insecure,
                prefer_openssl: opts.prefer_openssl,
                alt_name: None,
                dane_tlsa: vec![],
            })
            .await?;
        println!("{tls_result:?}");
    }

    Ok(())
}
