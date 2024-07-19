use anyhow::Context;
use chrono::Utc;
use clap::Parser;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use nix::sys::resource::{getrlimit, setrlimit, Resource};
use num_format::{Locale, ToFormattedString};
use once_cell::sync::OnceCell;
use rfc5321::*;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
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

    /// When generating the body, use at least this
    /// many bytes of nonsense words
    #[arg(long, default_value = "1024")]
    body_size: humanize_rs::bytes::Bytes,

    #[arg(skip)]
    body_size_content: OnceCell<String>,
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

    async fn make_client(&self) -> anyhow::Result<SmtpClient> {
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
        let mine = counter.fetch_add(1, Ordering::SeqCst);
        if let Some(limit) = self.message_count {
            if mine >= limit {
                counter.fetch_sub(1, Ordering::SeqCst);
                return false;
            }
        }
        true
    }

    fn release_one(&self, counter: &Arc<AtomicUsize>) {
        counter.fetch_sub(1, Ordering::SeqCst);
    }

    async fn run(&self, counter: Arc<AtomicUsize>) -> anyhow::Result<()> {
        let start = Instant::now();
        'reconnect: while !self.done(start, &counter) {
            let mut client = self.make_client().await?;
            while !self.done(start, &counter) {
                if !self.claim_one(&counter) {
                    client.send_command(&Command::Quit).await?;
                    return Ok(());
                }
                let (sender, recip, body) = self.generate_message();
                match client.send_mail(sender, recip, body).await
                {
                    Ok(_) => {
                    }
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
                        self.release_one(&counter);
                        continue 'reconnect;
                    }
                    Err(err) => {
                        self.release_one(&counter);
                        return Err(err).context("Error sending mail");
                    }
                };
            }
            client.send_command(&Command::Quit).await?;
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

    let mut clients = FuturesUnordered::new();
    for _ in 0..concurrency {
        let opts = opts.clone();
        let counter = Arc::clone(&counter);
        clients.push(tokio::spawn(async move {
            if let Err(err) = opts.run(counter).await {
                eprintln!("\n{err:#}");
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

    let deadline = tokio::time::Instant::now() + duration;
    let update_interval = Duration::from_secs(1);
    let mut last_update_time = Instant::now();
    let mut last_sent = 0;

    #[allow(dead_code)]
    struct Rates {
        msgs_per_second: usize,
        msgs_per_minute: usize,
        msgs_per_hour: usize,
        per_minute: String,
        per_hour: String,
    }

    impl Rates {
        fn new(total_sent: usize, elapsed: Duration) -> Self {
            let msgs_per_second = (total_sent as f64 / elapsed.as_secs_f64()) as usize;
            let msgs_per_minute = msgs_per_second * 60;
            let msgs_per_hour = msgs_per_minute * 60;

            let per_minute = msgs_per_minute.to_formatted_string(&Locale::en);
            let per_hour = msgs_per_hour.to_formatted_string(&Locale::en);

            Self {
                msgs_per_second,
                msgs_per_minute,
                msgs_per_hour,
                per_minute,
                per_hour,
            }
        }

        fn print(&self, prefix: &str, suffix: &str) {
            let mut out = std::io::stdout();
            write!(out,
                "{prefix}{per_second} msgs/s, {per_minute} msgs/minute, {per_hour} msgs/hour{suffix}",
                per_second = self.msgs_per_second,
                per_minute = self.per_minute,
                per_hour = self.per_hour
            ).unwrap();
            out.flush().unwrap();
        }
    }

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
                    &format!(" (sent={total_sent}, clients={running_clients})"));
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

    println!("did {total_sent} messages over {elapsed:?}.");
    rates.print("overall rate: ", "");
    println!();

    Ok(())
}
