use mail_auth::{Resolver, MX};
use message::Message;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Receiver, Sender};
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
        let handle = manager.sites.entry(name.clone()).or_insert_with(|| {
            let (tx, rx) = tokio::sync::mpsc::channel(1024 /* FIXME: configurable */);
            SiteHandle(Arc::new(Mutex::new(DestinationSite {
                name: name.clone(),
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
    tx: Sender<Message>,
    rx: Receiver<Message>,
}

impl DestinationSite {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn insert(&self, msg: Message) -> Result<(), TrySendError<Message>> {
        self.tx.try_send(msg)
    }
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
}
