use mail_auth::{Resolver, MX};
use message::Message;
use ringbuf::{HeapRb, Rb};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch::{Receiver, Sender};
use tokio::sync::{Mutex, MutexGuard};

lazy_static::lazy_static! {
    static ref MANAGER: Mutex<SiteManager> = Mutex::new(SiteManager::new());
    static ref RESOLVER: Mutex<Resolver> = Mutex::new(Resolver::new_system_conf().unwrap());
}

pub struct SiteManager {
    sites: HashMap<String, SiteHandle>,
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
        let resolver = RESOLVER.lock().await;
        let mx = resolver
            .mx_lookup(name)
            .await
            .map_err(|err| anyhow::anyhow!("MX lookup for {name} failed: {err:#}"))?;
        let name = factor_mx_list(&mx);

        let mut manager = Self::get().await;
        let max_ready_items = 1024; // FIXME: configurable
        let handle = manager.sites.entry(name.clone()).or_insert_with(|| {
            let ready = HeapRb::new(max_ready_items);
            let (tx, rx) = tokio::sync::watch::channel(());
            SiteHandle(Arc::new(Mutex::new(DestinationSite {
                name: name.clone(),
                ready,
                mx,
                tx,
                rx,
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
    mx: Arc<Vec<MX>>,
    ready: HeapRb<Message>,
    tx: Sender<()>,
    rx: Receiver<()>,
}

impl DestinationSite {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn insert(&mut self, msg: Message) -> Result<(), Message> {
        self.ready.push(msg)?;
        self.tx.send(()).ok();
        Ok(())
    }

    pub fn ready_count(&self) -> usize {
        self.ready.len()
    }

    pub fn ideal_connection_count(&self) -> usize {
        let connection_limit = 32; // TODO: configurable
        ideal_connection_count(self.ready_count(), connection_limit)
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

/// Given a set of MX records, produce a pseudo-regex style alternation
/// list of the underlying hostnames
fn factor_mx_list(mx: &Arc<Vec<MX>>) -> String {
    let mut names = vec![];
    for entry in mx.iter() {
        for host in &entry.exchanges {
            names.push(host.as_str());
        }
    }

    factor_names(&names)
}

/// Given a list of host names, produce a pseudo-regex style alternation list
/// of the different elements of the hostnames.
/// The goal is to produce a more compact representation of the name list
/// with the common components factored out.
fn factor_names(names: &[&str]) -> String {
    let mut max_element_count = 0;

    let mut elements: Vec<Vec<&str>> = vec![];

    let mut split_names = vec![];
    for name in names {
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
