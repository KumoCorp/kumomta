use anyhow::Context;
use gcd::Gcd;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

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
    // TODO: ha proxy cluster protocol options to go here
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
