use crate::labels::Labels;
use prometheus::core::{Collector, Desc};
use prometheus::proto::MetricFamily;
use prometheus::{IntGauge, IntGaugeVec, Opts};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

/// Vends out labelled IntGauges that will automatically remove themselves
/// when they are no longer referenced.
#[derive(Clone)]
pub struct PruningIntGaugeVec {
    gauges: IntGaugeVec,
    labels: Arc<Mutex<HashMap<Labels, Weak<()>>>>,
}

impl PruningIntGaugeVec {
    pub fn register(name: &str, help: &str, label_names: &[&str]) -> Self {
        let me = Self {
            gauges: IntGaugeVec::new(Opts::new(name, help), label_names)
                .expect("create IntGaugeVec failed"),
            labels: Arc::new(Mutex::new(HashMap::new())),
        };

        prometheus::register(Box::new(me.clone())).expect("register PruningIntGaugeVec failed");

        me
    }

    pub fn with_label_values(&self, labels: &[&str]) -> PruningIntGauge {
        let label_key = Labels::new(labels);
        let mut label_mgr = self.labels.lock().unwrap();
        let label_ref = match label_mgr.get(&label_key).and_then(|weak| weak.upgrade()) {
            Some(entry) => entry,
            None => {
                let strong = Arc::new(());
                label_mgr.insert(label_key, Arc::downgrade(&strong));
                strong
            }
        };

        PruningIntGauge {
            gauge: self.gauges.with_label_values(labels),
            _label_ref: label_ref,
        }
    }

    /// The values in the labels map are Weak refs to the corresponding
    /// strong ref maintained in the wrapped individual counter that we
    /// hand out.
    /// The weak ref is upgradable to the strong ref while there are
    /// any live counter references in existence, but not when they
    /// are all out of scope.
    /// This prune method uses that fact to prune out unreachable
    /// metrics, reducing the size of the generated metrics for
    /// the endpoint.
    pub fn prune_dead(&self) {
        self.labels
            .lock()
            .unwrap()
            .retain(|labels, weak| match weak.upgrade() {
                Some(_) => true,
                None => {
                    self.gauges
                        .remove_label_values(&labels.labels_ref())
                        .unwrap();
                    false
                }
            });
    }
}

impl Collector for PruningIntGaugeVec {
    fn desc(&self) -> Vec<&Desc> {
        self.gauges.desc()
    }
    fn collect(&self) -> Vec<MetricFamily> {
        self.prune_dead();
        self.gauges.collect()
    }
}

#[derive(Clone)]
pub struct PruningIntGauge {
    gauge: IntGauge,
    _label_ref: Arc<()>,
}

impl std::ops::Deref for PruningIntGauge {
    type Target = IntGauge;

    fn deref(&self) -> &IntGauge {
        &self.gauge
    }
}
