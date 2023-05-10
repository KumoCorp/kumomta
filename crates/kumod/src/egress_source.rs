use anyhow::Context;
use gcd::Gcd;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpSocket, TcpStream};

lazy_static::lazy_static! {
    static ref SOURCES: Mutex<HashMap<String, EgressSource>> = Mutex::new(EgressSource::init_sources());
    static ref POOLS: Mutex<HashMap<String, EgressPool>> = Mutex::new(EgressPool::init_pools());
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct EgressSource {
    /// Give it a friendly name for use in reporting and referencing
    /// elsewhere in the config
    pub name: String,

    /// Bind to this local address prior to issuing a connect(2) syscall
    pub source_address: Option<IpAddr>,

    /// Override the default destination port number with this value
    /// for deployments that use port mapping
    pub remote_port: Option<u16>,

    /// The host:port of the haproxy that should be used
    pub ha_proxy_server: Option<SocketAddr>,

    /// Ask ha_proxy to bind to this address when it is making
    /// a connection
    pub ha_proxy_source_address: Option<IpAddr>,
}

impl EgressSource {
    pub fn register(&self) {
        SOURCES
            .lock()
            .unwrap()
            .insert(self.name.to_string(), self.clone());
    }

    fn init_sources() -> HashMap<String, Self> {
        let mut map = HashMap::new();
        let unspec = Self {
            name: "unspecified".to_string(),
            source_address: None,
            remote_port: None,
            ha_proxy_server: None,
            ha_proxy_source_address: None,
        };

        map.insert(unspec.name.to_string(), unspec);
        map
    }
    pub fn resolve(name: &str) -> anyhow::Result<Self> {
        SOURCES
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no such source {name}"))
    }

    pub async fn connect_to(&self, address: SocketAddr) -> anyhow::Result<TcpStream> {
        use ppp::v2::{Addresses, Builder, Command, IPv4, IPv6, Protocol, Version};
        let source_name = &self.name;

        let (ha_proxy_server, ha_proxy_addr) =
            match (self.ha_proxy_server, self.ha_proxy_source_address) {
                (Some(srv), Some(addr)) => match (addr, address) {
                    (IpAddr::V4(src_ip), SocketAddr::V4(dest_ip)) => (
                        Some(srv),
                        Some(Addresses::IPv4(IPv4::new(
                            src_ip,
                            *dest_ip.ip(),
                            0,
                            dest_ip.port(),
                        ))),
                    ),
                    (IpAddr::V6(src_ip), SocketAddr::V6(dest_ip)) => (
                        Some(srv),
                        Some(Addresses::IPv6(IPv6::new(
                            src_ip,
                            *dest_ip.ip(),
                            0,
                            dest_ip.port(),
                        ))),
                    ),
                    _ => anyhow::bail!(
                        "Skipping {source_name} because \
                         ha_proxy_source_address address family does \
                         not match the destination address family"
                    ),
                },
                _ => (None, None),
            };

        let transport_address = ha_proxy_server.unwrap_or(address);

        tracing::trace!(
            "will connect {address:?} {transport_address:?} {ha_proxy_server:?} {ha_proxy_addr:?}"
        );

        let socket = match transport_address {
            SocketAddr::V4(_) => TcpSocket::new_v4(),
            SocketAddr::V6(_) => TcpSocket::new_v6(),
        }
        .with_context(|| format!("make socket to connect to {transport_address:?}"))?;

        if let Some(source) = self.source_address {
            if let Err(err) = socket.bind(SocketAddr::new(source, 0)) {
                // Always log failure to bind: it indicates a critical
                // misconfiguration issue
                let error = format!(
                    "bind {source:?} for source:{source_name} failed: {err:#} \
                    while attempting to connect to {transport_address:?}"
                );
                tracing::error!("{error}");
                anyhow::bail!("{error}");
            }
        }
        let mut stream = socket
            .connect(transport_address)
            .await
            .with_context(|| format!("connect to {transport_address:?}"))?;

        if let Some(proxy_addr) = ha_proxy_addr {
            let header = Builder::with_addresses(
                Version::Two | Command::Proxy,
                Protocol::Stream,
                proxy_addr,
            )
            .build()
            .with_context(|| {
                format!(
                    "building ha proxy protocol header \
                     for connection from source:{source_name} to {address:?}"
                )
            })?;

            stream.write_all(&header).await.with_context(|| {
                format!(
                    "sending ha proxy protocol header \
                     for connection from source:{source_name} to {address:?}"
                )
            })?;
        }

        Ok(stream)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct EgressPoolEntry {
    /// Name of an EgressSource to include in this pool
    pub name: String,

    /// Weight of this entry in the pool. If all entries have the same
    /// weight, then they have equal chance of being selected.
    /// If one entry has a weight that is twice that of the other
    /// entry in the pool, then it is twice as likely to be selected
    /// as the other one.
    ///
    /// A weight of 0 prevents this entry from being used.
    #[serde(default = "EgressPoolEntry::default_weight")]
    pub weight: u32,
}

impl EgressPoolEntry {
    fn default_weight() -> u32 {
        1
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct EgressPool {
    /// Name of the pool
    pub name: String,

    /// and the sources the constitute this pool
    pub entries: Vec<EgressPoolEntry>,
}

impl EgressPool {
    pub fn register(&self) -> anyhow::Result<()> {
        for entry in &self.entries {
            EgressSource::resolve(&entry.name)
                .with_context(|| format!("defining egress pool {}", self.name))?;
        }
        POOLS
            .lock()
            .unwrap()
            .insert(self.name.to_string(), self.clone());
        Ok(())
    }

    pub fn resolve(name: Option<&str>) -> anyhow::Result<Self> {
        let name = name.unwrap_or("unspecified");
        POOLS
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no such pool {name}"))
    }

    fn init_pools() -> HashMap<String, Self> {
        let mut map = HashMap::new();
        let unspec = Self {
            name: "unspecified".to_string(),
            entries: vec![EgressPoolEntry {
                name: "unspecified".to_string(),
                weight: 1,
            }],
        };

        map.insert(unspec.name.to_string(), unspec);
        map
    }
}

/// Maintains the state to manage Weighted Round Robin
/// <http://kb.linuxvirtualserver.org/wiki/Weighted_Round-Robin_Scheduling>
pub struct EgressPoolRoundRobin {
    pub name: String,
    entries: Vec<EgressPoolEntry>,
    max_weight: u32,
    gcd: u32,

    current_index: usize,
    current_weight: u32,
}

impl EgressPoolRoundRobin {
    pub fn new(pool: &EgressPool) -> Self {
        let mut entries = vec![];
        let mut max_weight = 0;
        let mut gcd = 0;

        for entry in &pool.entries {
            if entry.weight == 0 {
                continue;
            }
            max_weight = max_weight.max(entry.weight);
            gcd = gcd.gcd(entry.weight);
            entries.push(entry.clone());
        }

        Self {
            name: pool.name.to_string(),
            entries,
            max_weight,
            gcd,
            current_index: 0,
            current_weight: 0,
        }
    }

    pub fn all_sources(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|ent| ent.name.to_string())
            .collect()
    }

    pub fn next(&mut self) -> Option<String> {
        if self.entries.is_empty() || self.max_weight == 0 {
            return None;
        }
        if self.entries.len() == 1 {
            return self.entries.get(0).map(|entry| entry.name.to_string());
        }
        loop {
            self.current_index = (self.current_index + 1) % self.entries.len();
            if self.current_index == 0 {
                self.current_weight = self.current_weight.saturating_sub(self.gcd);
                if self.current_weight == 0 {
                    self.current_weight = self.max_weight;
                }
            }

            if let Some(entry) = self.entries.get(self.current_index) {
                if entry.weight >= self.current_weight {
                    return Some(entry.name.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn round_robin() {
        let pool = EgressPool {
            name: "pool".to_string(),
            entries: vec![
                EgressPoolEntry {
                    name: "one".to_string(),
                    weight: 5,
                },
                EgressPoolEntry {
                    name: "two".to_string(),
                    weight: 2,
                },
                EgressPoolEntry {
                    name: "three".to_string(),
                    weight: 3,
                },
            ],
        };

        let mut rr = EgressPoolRoundRobin::new(&pool);
        let mut counts = HashMap::new();

        for _ in 0..100 {
            let name = rr.next().unwrap();
            *counts.entry(name).or_insert(0) += 1;
        }

        // Counts should be in the same proportion as the
        // weights of the respective pools
        assert_eq!(counts["one"], 50, "one");
        assert_eq!(counts["two"], 20, "two");
        assert_eq!(counts["three"], 30, "three");
    }
}
