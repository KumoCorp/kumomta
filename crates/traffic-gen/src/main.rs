use anyhow::Context;
use chrono::Utc;
use clap::builder::ValueParser;
use clap::Parser;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use hdrhistogram::sync::Recorder;
use hdrhistogram::Histogram;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use num_format::{Locale, ToFormattedString};
use once_cell::sync::OnceCell;
use reqwest::{Client as HttpClient, Url};
use rfc5321::*;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use throttle::ThrottleSpec;
use tokio::net::TcpStream;
use uuid::Uuid;

const DOMAINS: &[&str] = &["aol.com", "gmail.com", "hotmail.com", "yahoo.com"];

#[derive(Clone, Debug, Parser)]
#[command(about = "SMTP traffic generator")]
struct Opt {
    /// All generated mail will have this domain appended.
    /// The default is an MX that routes to a loopback address.
    #[arg(long, default_value = "mx-sink.wezfurlong.org")]
    domain_suffix: String,

    /// The target host to which mail will be submitted
    #[arg(long, default_value = "127.0.0.1:2025")]
    target: String,

    /// The number of connections to open to target
    #[arg(long)]
    concurrency: Option<usize>,

    /// How many seconds to generate for
    #[arg(long, default_value = "60")]
    duration: u64,

    /// Rather than generate as many messages as we can
    /// within the specified duration, generate exactly
    /// this many messages
    #[arg(long)]
    message_count: Option<usize>,

    /// Whether to use STARTTLS for submission
    #[arg(long)]
    starttls: bool,

    /// Take the message contents from the specified file
    #[arg(long)]
    body_file: Option<PathBuf>,

    #[arg(skip)]
    body_file_content: OnceCell<String>,

    /// Include this domain in the list of domains for which mail
    /// will be generated.
    #[arg(long)]
    domain: Option<Vec<String>>,

    /// Limit the sending rate to the specified rate
    #[arg(long, value_parser=ValueParser::new(parse_throttle))]
    throttle: Option<ThrottleSpec>,

    /// When generating the body, use at least this
    /// many bytes of nonsense words
    #[arg(long, default_value = "1024")]
    body_size: humanize_rs::bytes::Bytes,

    #[arg(skip)]
    body_size_content: OnceCell<String>,

    /// Use http injection API instead of SMTP
    #[arg(long)]
    http: bool,

    /// When using http injection, enable deferred_spool
    #[arg(long)]
    http_defer_spool: bool,

    /// When using http injection, enable deferred_generation
    #[arg(long)]
    http_defer_generation: bool,

    /// When using http injection, how many recipients to generate
    /// in a single request
    #[arg(long, default_value = "1")]
    http_batch_size: usize,
}

fn parse_throttle(arg: &str) -> Result<ThrottleSpec, String> {
    ThrottleSpec::try_from(arg)
}

struct InjectClient {
    url: Url,
    client: HttpClient,
    defer_spool: bool,
    defer_generation: bool,
    batch_size: usize,
}

impl InjectClient {
    async fn send_mail(
        &mut self,
        sender: ReversePath,
        recip: ForwardPath,
        body: String,
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Recipient {
            email: String,
        }

        #[derive(Serialize)]
        struct InjectRequest {
            content: String,
            envelope_sender: String,
            recipients: Vec<Recipient>,
            deferred_spool: bool,
            deferred_generation: bool,
        }

        let mut recipients = vec![];
        for _ in 0..self.batch_size {
            recipients.push(Recipient {
                email: recip.to_string(),
            });
        }

        let response = self
            .client
            .request(reqwest::Method::POST, self.url.clone())
            .json(&InjectRequest {
                content: body,
                envelope_sender: sender.to_string(),
                recipients,
                deferred_spool: self.defer_spool,
                deferred_generation: self.defer_generation,
            })
            .send()
            .await?;
        let status = response.status();
        let body_bytes = response.bytes().await.with_context(|| {
            format!(
                "request status {}: {}, and failed to read response body",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })?;
        let body_text = String::from_utf8_lossy(&body_bytes);
        if !status.is_success() {
            anyhow::bail!(
                "request status {}: {}. Response body: {body_text}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
            );
        }
        Ok(())
    }
}

enum SendDisposition {
    Ok,
    Reconnect,
    Failed(anyhow::Error),
}

enum Client {
    Smtp(SmtpClient),
    Http(InjectClient),
}

impl Client {
    async fn disconnect(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Smtp(client) => {
                client.send_command(&Command::Quit).await?;
                Ok(())
            }
            Self::Http(_) => Ok(()),
        }
    }

    async fn send_mail(
        &mut self,
        sender: ReversePath,
        recip: ForwardPath,
        body: String,
    ) -> SendDisposition {
        match self {
            Self::Smtp(client) => {
                let result = client.send_mail(sender, recip, body).await;
                match result
                {
                    Ok(_) => SendDisposition::Ok,
                    Err(ClientError::Io(_) |
                        ClientError::Rejected(Response { code: 421, .. }) |
                        ClientError::TimeOutResponse{..} |
                        ClientError::TimeOutRequest{..} |
                        ClientError::TimeOutData |
                        ClientError::Rejected(Response {
                        code: 451,
                        enhanced_code:
                            // Too many recipients
                            Some(EnhancedStatusCode {
                                class: 4,
                                subject: 5,
                                detail: 3,
                            }),
                        ..
                    })) => {
                        SendDisposition::Reconnect
                    }
                    err @ Err(_) => {
                        SendDisposition::Failed(err.context("Failed to send mail").unwrap_err())
                    }
                }
            }
            Self::Http(client) => match client.send_mail(sender, recip, body).await {
                Ok(_) => SendDisposition::Ok,
                Err(err) => SendDisposition::Failed(err),
            },
        }
    }
}

impl Opt {
    fn pick_a_domain(&self) -> String {
        let number: usize = rand::random();
        let domain = match &self.domain {
            Some(domains) => domains[number % domains.len()].as_str(),
            None => DOMAINS[number % DOMAINS.len()],
        };
        if self.domain_suffix.is_empty() {
            return domain.to_string();
        }
        format!("{domain}.{}", self.domain_suffix)
    }

    fn generate_sender(&self) -> String {
        format!("noreply@{}", self.pick_a_domain())
    }

    fn generate_recipient(&self) -> String {
        let number: usize = rand::random();
        let domain = self.pick_a_domain();
        format!("user-{number}@{domain}")
    }

    fn load_body_file(&self) -> anyhow::Result<()> {
        if let Some(path) = &self.body_file {
            let data =
                std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
            // Canonicalize the line endings
            let data = data.replace("\r\n", "\n").replace("\n", "\r\n");
            self.body_file_content.set(data).unwrap();
        } else {
            self.body_size_content
                .set(self.generate_message_text())
                .unwrap();
        }

        Ok(())
    }

    /// Generate text suitable for use as a message body of at
    /// least n_bytes, wrapped to wrap columns using CRLF as
    /// the line endings
    fn generate_message_text(&self) -> String {
        let mut chain = lipsum::MarkovChain::new();
        chain.learn(lipsum::LIBER_PRIMUS);
        let mut result = String::new();
        for word in chain.iter() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(word);
            if result.len() >= self.body_size.size() {
                eprintln!(
                    "Generated body of size {}",
                    humansize::format_size(result.len(), humansize::DECIMAL)
                );
                let mut result = textwrap::fill(
                    &result,
                    textwrap::Options::new(78).line_ending(textwrap::LineEnding::CRLF),
                );
                result.push_str("\r\n");
                return result;
            }
        }
        unreachable!();
    }

    fn generate_body(&self, sender: &str, recip: &str) -> String {
        if let Some(content) = self.body_file_content.get() {
            return content.to_string();
        }

        let now = Utc::now();
        let datestamp = now.to_rfc2822();
        let id = Uuid::new_v4().simple().to_string();

        let body = self.body_size_content.get().unwrap();

        format!(
            "From: <{sender}>\r\n\
             To: <{recip}>\r\n\
             Subject: test {datestamp}\r\n\
             Message-Id: {id}\r\n\
             X-Mailer: KumoMta traffic-gen\r\n\
             \r\n\
             {body}"
        )
    }

    fn generate_message(&self) -> (ReversePath, ForwardPath, String) {
        let sender = self.generate_sender();
        let recip = self.generate_recipient();
        let body = self.generate_body(&sender, &recip);
        (
            ReversePath::try_from(sender.as_str()).unwrap(),
            ForwardPath::try_from(recip.as_str()).unwrap(),
            body,
        )
    }

    async fn make_client(&self) -> anyhow::Result<Client> {
        if self.http {
            self.make_http_client()
                .await
                .map(|client| Client::Http(client))
        } else {
            self.make_smtp_client()
                .await
                .map(|client| Client::Smtp(client))
        }
    }

    async fn make_http_client(&self) -> anyhow::Result<InjectClient> {
        let client = HttpClient::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        let url = if self.target == "127.0.0.1:2025" {
            "http://127.0.0.1:8000"
        } else {
            &self.target
        };
        let url = Url::parse(&format!("{url}/api/inject/v1"))?;

        Ok(InjectClient {
            client,
            url,
            defer_spool: self.http_defer_spool,
            defer_generation: self.http_defer_generation,
            batch_size: self.http_batch_size,
        })
    }

    async fn make_smtp_client(&self) -> anyhow::Result<SmtpClient> {
        let timeouts = SmtpClientTimeouts::default();

        let stream =
            tokio::time::timeout(timeouts.connect_timeout, TcpStream::connect(&self.target))
                .await
                .with_context(|| format!("timed out connecting to {}", self.target))?
                .with_context(|| format!("failed to connect to {}", self.target))?;
        let mut client = SmtpClient::with_stream(stream, &self.target, timeouts);

        // Read banner
        let banner_timeout = client.timeouts().banner_timeout;
        let banner = client.read_response(None, banner_timeout).await?;
        anyhow::ensure!(banner.code == 220, "unexpected banner: {banner:#?}");

        // Say EHLO
        let caps = client.ehlo(&self.pick_a_domain()).await?;

        if self.starttls && caps.contains_key("STARTTLS") {
            client
                .starttls(TlsOptions {
                    insecure: true,
                    prefer_openssl: false,
                    alt_name: None,
                    dane_tlsa: vec![],
                    ..Default::default()
                })
                .await?;
        }

        Ok(client)
    }

    fn done(&self, start: Instant, counter: &Arc<AtomicUsize>) -> bool {
        if let Some(limit) = self.message_count {
            return counter.load(Ordering::SeqCst) >= limit;
        }
        start.elapsed() >= Duration::from_secs(self.duration)
    }

    fn claim_one(&self, counter: &Arc<AtomicUsize>) -> bool {
        let n = if self.http { self.http_batch_size } else { 1 };
        let mine = counter.fetch_add(n, Ordering::SeqCst);
        if let Some(limit) = self.message_count {
            if mine >= limit {
                counter.fetch_sub(n, Ordering::SeqCst);
                return false;
            }
        }
        true
    }

    fn release_one(&self, counter: &Arc<AtomicUsize>) {
        let n = if self.http { self.http_batch_size } else { 1 };
        counter.fetch_sub(n, Ordering::SeqCst);
    }

    async fn run(
        &self,
        counter: Arc<AtomicUsize>,
        mut latency: Recorder<u64>,
    ) -> anyhow::Result<()> {
        let start = Instant::now();
        'reconnect: while !self.done(start, &counter) {
            let mut client = self.make_client().await?;
            while !self.done(start, &counter) {
                if !self.claim_one(&counter) {
                    client.disconnect().await?;
                    return Ok(());
                }

                if let Some(spec) = &self.throttle {
                    loop {
                        let result = spec.throttle("send").await?;
                        if let Some(delay) = result.retry_after {
                            tokio::time::sleep(delay).await;
                        } else {
                            break;
                        }
                    }
                }

                let (sender, recip, body) = self.generate_message();
                let start = Instant::now();
                let result = client.send_mail(sender, recip, body).await;
                latency.record(start.elapsed().as_micros() as u64).ok();

                match result {
                    SendDisposition::Ok => {}
                    SendDisposition::Reconnect => {
                        self.release_one(&counter);
                        continue 'reconnect;
                    }
                    SendDisposition::Failed(err) => {
                        self.release_one(&counter);
                        return Err(err);
                    }
                };
            }
            client.disconnect().await?;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opt::parse();
    opts.load_body_file()?;

    let (_no_file_soft, no_file_hard) = getrlimit(Resource::RLIMIT_NOFILE)?;
    setrlimit(Resource::RLIMIT_NOFILE, no_file_hard, no_file_hard)?;

    let counter = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let duration = Duration::from_secs(opts.duration);
    let concurrency = match opts.concurrency {
        Some(n) => n,
        None => {
            let n_threads: usize = std::thread::available_parallelism()?.into();
            n_threads * 10
        }
    };

    let mut latency = Histogram::new(3)?.into_sync();

    let mut clients = FuturesUnordered::new();
    for _ in 0..concurrency {
        let opts = opts.clone();
        let counter = Arc::clone(&counter);
        let latency = latency.recorder();
        clients.push(tokio::spawn(async move {
            if let Err(err) = opts.run(counter, latency).await {
                eprintln!("\n{err:#}");
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

    let deadline = tokio::time::Instant::now() + duration;
    let update_interval = Duration::from_secs(1);
    let mut last_update_time = Instant::now();
    let mut last_sent = 0;

    let mut running_clients = concurrency;

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                println!("\nDeadline reached, stopping");
                break;
            },
            _ = tokio::time::sleep(update_interval) => {
                let now = Instant::now();
                let total_sent = counter.load(Ordering::Acquire);
                let elapsed = now - last_update_time;
                last_update_time = now;

                let rate = Rates::new(total_sent - last_sent, elapsed);
                rate.print("\r\x1b[Kcurrent rate: ",
                    &format!(" (sent={total_sent}, clients={running_clients})",
                    total_sent=total_sent.to_formatted_string(&Locale::en),
                    running_clients=running_clients.to_formatted_string(&Locale::en)));
                last_sent = total_sent;
            },
            item = clients.next() => {
                running_clients -= 1;
                if item.is_none(){
                    println!("\nAll clients finished");
                    break;
                }
            }
        };
    }

    let total_sent = counter.load(Ordering::Acquire);
    let elapsed = started.elapsed();
    let rates = Rates::new(total_sent, elapsed);

    println!(
        "did {total_sent} messages over {elapsed:?}.",
        total_sent = total_sent.to_formatted_string(&Locale::en)
    );

    print!("transaction latency: ");
    latency.refresh();
    print!("avg={:?} ", Duration::from_micros(latency.mean() as u64));
    print!("min={:?} ", Duration::from_micros(latency.min()));
    print!("max={:?} ", Duration::from_micros(latency.max()));
    for p in [50., 75., 90.0, 95., 99.0, 99.9] {
        let duration = Duration::from_micros(latency.value_at_quantile(p / 100.));
        print!("p{p}={duration:?} ",);
    }
    println!();

    rates.print("overall rate: ", "");
    println!();

    Ok(())
}

#[allow(dead_code)]
struct Rates {
    msgs_per_second: usize,
    msgs_per_minute: usize,
    msgs_per_hour: usize,
    per_second: String,
    per_minute: String,
    per_hour: String,
}

impl Rates {
    fn new(total_sent: usize, elapsed: Duration) -> Self {
        let msgs_per_second = (total_sent as f64 / elapsed.as_secs_f64()) as usize;
        let msgs_per_minute = msgs_per_second * 60;
        let msgs_per_hour = msgs_per_minute * 60;

        let per_second = msgs_per_second.to_formatted_string(&Locale::en);
        let per_minute = msgs_per_minute.to_formatted_string(&Locale::en);
        let per_hour = msgs_per_hour.to_formatted_string(&Locale::en);

        Self {
            msgs_per_second,
            msgs_per_minute,
            msgs_per_hour,
            per_second,
            per_minute,
            per_hour,
        }
    }

    fn print(&self, prefix: &str, suffix: &str) {
        let mut out = std::io::stdout();
        write!(
            out,
            "{prefix}{per_second} msgs/s, {per_minute} msgs/minute, {per_hour} msgs/hour{suffix}",
            per_second = self.per_second,
            per_minute = self.per_minute,
            per_hour = self.per_hour
        )
        .unwrap();
        out.flush().unwrap();
    }
}
