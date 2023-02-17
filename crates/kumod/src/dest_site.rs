use crate::queue::QueueManager;
use crate::spool::SpoolManager;
use anyhow::Context;
use mail_auth::{IpLookupStrategy, Resolver};
use mail_send::smtp::message::Message as SendMessage;
use mail_send::smtp::AssertReply;
use mail_send::SmtpClient;
use message::Message;
use rfc5321::{AsyncReadAndWrite, BoxedAsyncReadAndWrite};
use ringbuf::{HeapRb, Rb};
use rustls::client::WebPkiVerifier;
use rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore};
use smtp_proto::Response;
use std::borrow::Cow;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, MutexGuard, Notify};
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;

lazy_static::lazy_static! {
    static ref MANAGER: Mutex<SiteManager> = Mutex::new(SiteManager::new());
    static ref RESOLVER: Mutex<Resolver> = Mutex::new(Resolver::new_system_conf().unwrap());
}

pub struct SiteManager {
    sites: HashMap<String, SiteHandle>,
}

async fn resolve_mx(domain_name: &str) -> anyhow::Result<Vec<String>> {
    let resolver = RESOLVER.lock().await;
    match resolver.mx_lookup(domain_name).await {
        Ok(mxs) if mxs.is_empty() => Ok(vec![domain_name.to_string()]),
        Ok(mxs) => {
            let mut hosts = vec![];
            for mx in mxs.iter() {
                let mut hosts_this_pref: Vec<String> =
                    mx.exchanges.iter().map(|s| s.to_string()).collect();
                hosts_this_pref.sort();
                hosts.append(&mut hosts_this_pref);
            }
            Ok(hosts)
        }
        err @ Err(mail_auth::Error::DnsRecordNotFound(_)) => {
            match resolver.exists(domain_name).await {
                Ok(true) => Ok(vec![domain_name.to_string()]),
                _ => anyhow::bail!("{:#}", err.unwrap_err()),
            }
        }
        Err(err) => anyhow::bail!("MX lookup for {domain_name} failed: {err:#}"),
    }
}

impl SiteManager {
    pub fn new() -> Self {
        Self {
            sites: HashMap::new(),
        }
    }

    pub async fn get() -> MutexGuard<'static, Self> {
        MANAGER.lock().await
    }

    pub async fn resolve_domain(name: &str) -> anyhow::Result<SiteHandle> {
        let mx = Arc::new(resolve_mx(name).await?.into_boxed_slice());
        let name = factor_names(&mx);

        let mut manager = Self::get().await;
        let max_ready_items = 1024; // FIXME: configurable
        let handle = manager.sites.entry(name.clone()).or_insert_with(|| {
            tokio::spawn({
                let name = name.clone();
                async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        let mut mgr = SiteManager::get().await;
                        let site = { mgr.sites.get(&name).cloned() };
                        match site {
                            None => break,
                            Some(site) => {
                                let mut site = site.lock().await;
                                if site.reapable() {
                                    tracing::debug!("idle out {name}");
                                    mgr.sites.remove(&name);
                                    break;
                                }
                            }
                        }
                    }
                }
            });

            let ready = Arc::new(StdMutex::new(HeapRb::new(max_ready_items)));
            let notify = Arc::new(Notify::new());
            SiteHandle(Arc::new(Mutex::new(DestinationSite {
                name: name.clone(),
                ready,
                mx,
                notify,
                connections: vec![],
                last_change: Instant::now(),
            })))
        });
        Ok(handle.clone())
    }
}

#[derive(Clone)]
pub struct SiteHandle(Arc<Mutex<DestinationSite>>);

impl SiteHandle {
    pub async fn lock(&self) -> MutexGuard<DestinationSite> {
        self.0.lock().await
    }
}

pub struct DestinationSite {
    name: String,
    mx: Arc<Box<[String]>>,
    ready: Arc<StdMutex<HeapRb<Message>>>,
    notify: Arc<Notify>,
    connections: Vec<JoinHandle<()>>,
    last_change: Instant,
}

impl DestinationSite {
    #[allow(unused)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn insert(&mut self, msg: Message) -> Result<(), Message> {
        self.ready.lock().unwrap().push(msg)?;
        self.notify.notify_waiters();
        self.maintain();
        self.last_change = Instant::now();

        Ok(())
    }

    pub fn ready_count(&self) -> usize {
        self.ready.lock().unwrap().len()
    }

    pub fn ideal_connection_count(&self) -> usize {
        let connection_limit = 32; // TODO: configurable
        ideal_connection_count(self.ready_count(), connection_limit)
    }

    pub fn maintain(&mut self) {
        // Prune completed connection tasks
        self.connections.retain(|handle| !handle.is_finished());

        // TODO: throttle rate at which connections are opened
        let ideal = self.ideal_connection_count();

        for _ in self.connections.len()..ideal {
            // Open a new connection
            let name = self.name.clone();
            let mx = self.mx.clone();
            let ready = Arc::clone(&self.ready);
            let notify = self.notify.clone();
            self.connections.push(tokio::spawn(async move {
                if let Err(err) = Dispatcher::run(&name, mx, ready, notify).await {
                    tracing::error!("Error in dispatch_queue for {name}: {err:#}");
                }
            }));
        }
    }

    pub fn reapable(&mut self) -> bool {
        self.maintain();
        let ideal = self.ideal_connection_count();
        ideal == 0
            && self.connections.is_empty()
            && self.last_change.elapsed() > Duration::from_secs(10 * 60)
    }
}

#[derive(Debug, Clone)]
struct ResolvedAddress {
    #[allow(dead_code)] // used when logging, but rust warns anyway
    mx_host: String,
    addr: IpAddr,
}
async fn resolve_addresses(mx: &Arc<Box<[String]>>) -> Vec<ResolvedAddress> {
    let mut result = vec![];

    for mx_host in mx.iter() {
        match RESOLVER
            .lock()
            .await
            .ip_lookup(mx_host, IpLookupStrategy::default(), 32)
            .await
        {
            Err(err) => {
                tracing::error!("failed to resolve {mx_host}: {err:#}");
                continue;
            }
            Ok(addresses) => {
                for addr in addresses {
                    result.push(ResolvedAddress {
                        mx_host: mx_host.to_string(),
                        addr,
                    });
                }
            }
        }
    }
    result.reverse();
    result
}

struct Dispatcher {
    name: String,
    ready: Arc<StdMutex<HeapRb<Message>>>,
    notify: Arc<Notify>,
    addresses: Vec<ResolvedAddress>,
    msg: Option<Message>,
    client: Option<SmtpClient<BoxedAsyncReadAndWrite>>,
    client_address: Option<ResolvedAddress>,
    ehlo_name: String,
}

impl Dispatcher {
    async fn run(
        name: &str,
        mx: Arc<Box<[String]>>,
        ready: Arc<StdMutex<HeapRb<Message>>>,
        notify: Arc<Notify>,
    ) -> anyhow::Result<()> {
        let ehlo_name = gethostname::gethostname()
            .to_str()
            .unwrap_or("[127.0.0.1]")
            .to_string();

        let addresses = resolve_addresses(&mx).await;
        let mut dispatcher = Self {
            name: name.to_string(),
            ready,
            notify,
            msg: None,
            client: None,
            client_address: None,
            addresses,
            ehlo_name,
        };

        dispatcher.obtain_message();
        if dispatcher.msg.is_none() {
            // We raced with another dispatcher and there is no
            // more work to be done; no need to open a new connection.
            return Ok(());
        }

        loop {
            if !dispatcher.wait_for_message().await? {
                // No more messages within our idle time; we can close
                // the connection
                tracing::debug!("{} Idling out connection", dispatcher.name);
                return Ok(());
            }
            if let Err(err) = dispatcher.attempt_connection().await {
                if dispatcher.addresses.is_empty() {
                    return Err(err);
                }
                tracing::error!("{err:#}");
                // Try the next candidate MX address
                continue;
            }
            dispatcher.deliver_message().await?;
        }
    }

    fn obtain_message(&mut self) -> bool {
        if self.msg.is_some() {
            return true;
        }
        self.msg = self.ready.lock().unwrap().pop();
        self.msg.is_some()
    }

    async fn wait_for_message(&mut self) -> anyhow::Result<bool> {
        if self.obtain_message() {
            return Ok(true);
        }

        let idle_timeout = Duration::from_secs(60); // TODO: configurable
        match tokio::time::timeout(idle_timeout, self.notify.notified()).await {
            Ok(()) => {}
            Err(_) => {}
        }
        Ok(self.obtain_message())
    }

    async fn attempt_connection(&mut self) -> anyhow::Result<()> {
        if self.client.is_some() {
            return Ok(());
        }

        let address = self
            .addresses
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no more addresses to try!"))?;

        let timeout = Duration::from_secs(60);
        let ehlo_name = self.ehlo_name.to_string();
        let mx_host = address.mx_host.to_string();

        let client: SmtpClient<Box<dyn AsyncReadAndWrite>> = tokio::time::timeout(timeout, {
            let address = address.clone();
            async move {
                let mut client = SmtpClient {
                    stream: TcpStream::connect((address.addr, 25))
                        .await
                        .with_context(|| format!("connect to {address:?} port 25"))?,
                    timeout,
                };

                // Read banner
                client
                    .read()
                    .await
                    .map_err(|err| anyhow::anyhow!("{err:#}"))?
                    .assert_positive_completion()
                    .map_err(|err| anyhow::anyhow!("{err:#}"))?;

                // Say EHLO
                let response = client
                    .ehlo(&ehlo_name)
                    .await
                    .map_err(|err| anyhow::anyhow!("{err:#}"))?;

                // Use STARTTLS if available.
                // We need to do some type erasure because SmtpClient is either
                // SmtpClient<TlsStream<TcpStream>> or SmtpClient<TcpStream>
                // depending on whether TLS is used or not.
                // We do a little dance to end up with SmtpClient<Box<dyn AsyncReadAndWrite>>>
                // in both cases.
                let boxed_stream: Box<dyn AsyncReadAndWrite> =
                    if response.has_capability(smtp_proto::EXT_START_TLS) {
                        let tls_connector = build_tls_connector();
                        let SmtpClient { stream, timeout: _ } = client
                            .start_tls(&tls_connector, &mx_host)
                            .await
                            .map_err(|err| anyhow::anyhow!("{err:#}"))?;
                        let boxed: Box<dyn AsyncReadAndWrite> = Box::new(stream);
                        boxed
                    } else {
                        let SmtpClient { stream, timeout: _ } = client;
                        let boxed: Box<dyn AsyncReadAndWrite> = Box::new(stream);
                        boxed
                    };
                Ok::<SmtpClient<Box<dyn AsyncReadAndWrite>>, anyhow::Error>(SmtpClient {
                    stream: boxed_stream,
                    timeout,
                })
            }
        })
        .await??;

        self.client.replace(client);
        self.client_address.replace(address);
        Ok(())
    }

    async fn requeue_message(msg: Message) -> anyhow::Result<()> {
        let mut queue_manager = QueueManager::get().await;
        msg.delay_by(Duration::from_secs(60));
        let domain = msg.recipient()?.domain().to_string();
        queue_manager.insert(&domain, msg).await?;
        Ok(())
    }

    async fn deliver_message(&mut self) -> anyhow::Result<()> {
        let data;
        let message = {
            let msg = self.msg.as_ref().unwrap();
            data = msg.get_data();

            SendMessage::new(
                msg.sender()?.to_string(),
                [msg.recipient()?.to_string()],
                Cow::Borrowed(&**data),
            )
        };

        match self.client.as_mut().unwrap().send(message).await {
            Err(mail_send::Error::UnexpectedReply(Response { code, esc, message }))
                if code >= 400 && code < 500 =>
            {
                // Transient failure
                if let Some(msg) = self.msg.take() {
                    Self::requeue_message(msg).await?;
                }
                tracing::debug!(
                    "failed to send message to {} {:?}: {code} {esc:?} {message}",
                    self.name,
                    self.client_address
                );
            }
            Err(mail_send::Error::UnexpectedReply(Response { code, esc, message })) => {
                tracing::error!(
                    "failed to send message to {} {:?}: {code} {esc:?} {message}",
                    self.name,
                    self.client_address
                );
                // FIXME: log permanent failure
                if let Some(msg) = self.msg.take() {
                    Self::remove_from_spool(msg).await?;
                }
                self.msg.take();
            }
            Err(err) => {
                // Transient failure; continue with another host
                tracing::error!(
                    "failed to send message to {} {:?}: {err:#}",
                    self.name,
                    self.client_address
                );
            }
            Ok(()) => {
                // FIXME: log success
                if let Some(msg) = self.msg.take() {
                    Self::remove_from_spool(msg).await?;
                }
                tracing::debug!("Delivered OK!");
            }
        };
        Ok(())
    }

    async fn remove_from_spool(msg: Message) -> anyhow::Result<()> {
        let id = *msg.id();
        let data_spool = SpoolManager::get_named("data").await?;
        let meta_spool = SpoolManager::get_named("meta").await?;
        let res_data = data_spool.lock().await.remove(id).await;
        let res_meta = meta_spool.lock().await.remove(id).await;
        if let Err(err) = res_data {
            tracing::error!("Error removing data for {id}: {err:#}");
        }
        if let Err(err) = res_meta {
            tracing::error!("Error removing meta for {id}: {err:#}");
        }
        Ok(())
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        // Ensure that we re-queue any message that we had popped
        if let Some(msg) = self.msg.take() {
            tokio::spawn(async move {
                if let Err(err) = Dispatcher::requeue_message(msg).await {
                    tracing::error!("error requeuing message: {err:#}");
                }
            });
        }
    }
}

/// Use an exponential decay curve in the increasing form, asymptotic up to connection_limit,
/// passes through 0.0, increasing but bounded to connection_limit.
///
/// Visualize on wolframalpha: "plot 32 * (1-exp(-x * 0.023)), x from 0 to 100, y from 0 to 32"
fn ideal_connection_count(queue_size: usize, connection_limit: usize) -> usize {
    let factor = 0.023;
    let goal = (connection_limit as f32) * (1. - (-1.0 * queue_size as f32 * factor).exp());
    goal.ceil() as usize
}

/// Given a list of host names, produce a pseudo-regex style alternation list
/// of the different elements of the hostnames.
/// The goal is to produce a more compact representation of the name list
/// with the common components factored out.
fn factor_names<S: AsRef<str>>(names: &[S]) -> String {
    let mut max_element_count = 0;

    let mut elements: Vec<Vec<&str>> = vec![];

    let mut split_names = vec![];
    for name in names {
        let name = name.as_ref();
        let mut fields: Vec<_> = name.split('.').map(|s| s.to_lowercase()).collect();
        fields.reverse();
        max_element_count = max_element_count.max(fields.len());
        split_names.push(fields);
    }

    fn add_element<'a>(elements: &mut Vec<Vec<&'a str>>, field: &'a str, i: usize) {
        match elements.get_mut(i) {
            Some(ele) => {
                if !ele.contains(&field) {
                    ele.push(field);
                }
            }
            None => {
                elements.push(vec![field]);
            }
        }
    }

    for fields in &split_names {
        for (i, field) in fields.iter().enumerate() {
            add_element(&mut elements, field, i);
        }
        for i in fields.len()..max_element_count {
            add_element(&mut elements, "?", i);
        }
    }

    let mut result = vec![];
    for mut ele in elements {
        let has_q = ele.contains(&"?");
        ele.retain(|&e| e != "?");
        let mut item_text = if ele.len() == 1 {
            ele[0].to_string()
        } else {
            format!("({})", ele.join("|"))
        };
        if has_q {
            item_text.push('?');
        }
        result.push(item_text);
    }
    result.reverse();

    result.join(".")
}

pub fn build_tls_connector() -> TlsConnector {
    let config = ClientConfig::builder().with_safe_defaults();

    let mut root_cert_store = RootCertStore::empty();

    root_cert_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    let config = config
        .with_custom_certificate_verifier(Arc::new(WebPkiVerifier::new(root_cert_store, None)))
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn name_factoring() {
        assert_eq!(
            factor_names(&[
                "mta5.am0.yahoodns.net",
                "mta6.am0.yahoodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net".to_string()
        );

        // Verify that the case is normalized to lowercase
        assert_eq!(
            factor_names(&[
                "mta5.AM0.yahoodns.net",
                "mta6.am0.yAHOodns.net",
                "mta7.am0.yahoodns.net"
            ]),
            "(mta5|mta6|mta7).am0.yahoodns.net".to_string()
        );

        // When the names have mismatched lengths, do we produce
        // something reasonable?
        assert_eq!(
            factor_names(&[
                "gmail-smtp-in.l.google.com",
                "alt1.gmail-smtp-in.l.google.com",
                "alt2.gmail-smtp-in.l.google.com",
                "alt3.gmail-smtp-in.l.google.com",
                "alt4.gmail-smtp-in.l.google.com",
            ]),
            "(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com".to_string()
        );
    }

    #[test]
    fn connection_limit() {
        let sizes = [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 32, 64, 128, 256, 400, 512, 1024,
        ];
        let max_connections = 32;
        let targets: Vec<(usize, usize)> = sizes
            .iter()
            .map(|&queue_size| {
                (
                    queue_size,
                    ideal_connection_count(queue_size, max_connections),
                )
            })
            .collect();
        assert_eq!(
            vec![
                (0, 0),
                (1, 1),
                (2, 2),
                (3, 3),
                (4, 3),
                (5, 4),
                (6, 5),
                (7, 5),
                (8, 6),
                (9, 6),
                (10, 7),
                (20, 12),
                (32, 17),
                (64, 25),
                (128, 31),
                (256, 32),
                (400, 32),
                (512, 32),
                (1024, 32)
            ],
            targets
        );
    }
}
