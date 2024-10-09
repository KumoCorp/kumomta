use anyhow::anyhow;
use bounce_classify::{
    BounceClass, BounceClassifier, BounceClassifierBuilder, PreDefinedBounceClass,
};
use config::epoch::{get_current_epoch, ConfigEpoch};
use kumo_log_types::{JsonLogRecord, RecordType};
use lru_cache::LruCache;
use parking_lot::Mutex;
use prometheus::Histogram;
use rfc5321::Response;
use serde::Deserialize;
use std::sync::{Arc, LazyLock, OnceLock};
use tokio::sync::oneshot;

static CLASSIFY_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "bounce_classify_latency",
        "latency of bounce classification",
    )
    .unwrap()
});
static CLASSIFY: OnceLock<ClassifierWrapper> = OnceLock::new();

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ClassifierParams {
    pub files: Vec<String>,

    #[serde(default = "ClassifierParams::default_back_pressure")]
    pub back_pressure: usize,

    #[serde(default = "ClassifierParams::default_pool_size")]
    pub pool_size: usize,

    #[serde(default = "ClassifierParams::default_cache_size")]
    pub cache_size: usize,

    #[serde(default = "ClassifierParams::default_cache_size")]
    pub uncategorized_cache_size: usize,
}

impl ClassifierParams {
    fn default_back_pressure() -> usize {
        128 * 1024
    }

    fn default_pool_size() -> usize {
        std::thread::available_parallelism()
            .map(|p| (p.get() / 4).max(1))
            .unwrap_or(4)
    }

    fn default_cache_size() -> usize {
        1024
    }

    fn load(&self) -> anyhow::Result<BounceClassifier> {
        let mut builder = BounceClassifierBuilder::new();
        for file_name in &self.files {
            if file_name.ends_with(".json") {
                builder
                    .merge_json_file(file_name)
                    .map_err(|err| anyhow!("{err}"))?;
            } else if file_name.ends_with(".toml") {
                builder
                    .merge_toml_file(file_name)
                    .map_err(|err| anyhow!("{err}"))?;
            } else {
                anyhow::bail!("{file_name}: classifier files must have either .toml or .json filename extension");
            }
        }

        builder.build().map_err(|err| anyhow!("{err}"))
    }

    pub fn register(&self) -> anyhow::Result<()> {
        let classifier = self.load()?;

        CLASSIFY
            .set(ClassifierWrapper::new(classifier, self)?)
            .map_err(|_| anyhow::anyhow!("classifier already initialized"))?;

        Ok(())
    }
}

struct ClassifyRequest {
    response: Response,
    tx: oneshot::Sender<BounceClass>,
    epoch: ConfigEpoch,
}

/// We maintain two caches; one for uncategorized results and the
/// other for categorized results. The rationale is that there
/// could be a large set of responses that do not have a categorization
/// and we don't want those to thrash and out-compete the actual
/// classifications and render the whole class completely ineffective.
struct State {
    cache: LruCache<Response, BounceClass>,
    uncat_cache: LruCache<Response, BounceClass>,
    classifier: Arc<BounceClassifier>,
    classifier_epoch: ConfigEpoch,
}

impl State {
    fn insert(&mut self, response: Response, result: BounceClass) {
        let cache = match &result {
            BounceClass::PreDefined(PreDefinedBounceClass::Uncategorized) => &mut self.uncat_cache,
            _ => &mut self.cache,
        };

        cache.insert(response.clone(), result.clone());
    }

    /// clear the caches and return a copy of the classifier.
    /// Intended to be called when one of the classifier threads detects
    /// that its locally cached classifier and epoch are outdated.
    /// The cache will be cleared by each thread, which guarantees that
    /// the last one to wake up and detect the change will ensure that
    /// no cached results from prior to the change are retained even
    /// in the face of uneven and splayed wakeups in the various
    /// classifier threads.
    ///
    /// The above only happens when the incoming epoch matches the epoch
    /// stored here in the state. This is because the incoming classification
    /// request comes from get_current_epoch which may be a later epoch
    /// than the one in the state: we may be racing with the subscriber
    /// to load and parse the updated config while traffic is incoming.
    ///
    /// Note that even with the cache clearing happening here, the
    /// reload process must also itself clear the cache in order
    /// for the workers to get woken up to call this method.
    fn get_updated_classifier(
        &mut self,
        current_epoch: ConfigEpoch,
    ) -> Option<Arc<BounceClassifier>> {
        if self.classifier_epoch == current_epoch {
            self.cache.clear();
            self.uncat_cache.clear();
            Some(self.classifier.clone())
        } else {
            None
        }
    }
}

struct ClassifierWrapper {
    tx: flume::Sender<ClassifyRequest>,
    state: Arc<Mutex<State>>,
}

impl ClassifierWrapper {
    fn new(classifier: BounceClassifier, params: &ClassifierParams) -> anyhow::Result<Self> {
        let classifier = Arc::new(classifier);
        let (tx, rx) = flume::bounded(params.back_pressure);

        let epoch = get_current_epoch();
        let state = Arc::new(Mutex::new(State {
            cache: LruCache::new(params.cache_size),
            uncat_cache: LruCache::new(params.uncategorized_cache_size),
            classifier: classifier.clone(),
            classifier_epoch: epoch,
        }));

        tracing::info!(
            "bounce-classify thread pool starting with {} threads",
            params.pool_size
        );
        for _ in 0..params.pool_size {
            let classifier = classifier.clone();
            let rx = rx.clone();
            let state = state.clone();
            std::thread::Builder::new()
                .name("bounce-classify".to_string())
                .spawn(move || {
                    let mut my_epoch = epoch;
                    let mut classifier = classifier;
                    while let Ok(ClassifyRequest {
                        response,
                        tx,
                        epoch,
                    }) = rx.recv()
                    {
                        tracing::trace!("classify request with {epoch:?}");
                        if epoch != my_epoch {
                            if let Some(c) = state.lock().get_updated_classifier(epoch) {
                                tracing::debug!("classifier applying new epoch {epoch:?}");
                                classifier = c;
                                my_epoch = epoch;
                            }
                        }

                        let result = classifier.classify_response(&response);
                        if epoch == my_epoch {
                            // Only cache if the epochs match up, as a cheap defensive
                            // measure to avoid poisoning the cache with a stale result
                            state.lock().insert(response.clone(), result.clone());
                        }
                        if tx.send(result).is_err() {
                            break;
                        }
                    }
                })?;
        }

        {
            let state = state.clone();
            let params = params.clone();
            tokio::spawn(async move {
                let mut subscriber = config::epoch::subscribe();
                let mut had_error = false;
                while let Ok(()) = subscriber.changed().await {
                    let epoch = *subscriber.borrow_and_update();

                    tracing::debug!("Reloading the bounce classifier for epoch {epoch:?}");
                    match params.load() {
                        Ok(classifier) => {
                            if had_error {
                                tracing::info!("Successfully loaded updated bounce classifier after previous error");
                                had_error = false;
                            }
                            let mut state = state.lock();
                            state.classifier = Arc::new(classifier);
                            state.classifier_epoch = epoch;
                            // and invalidate caches in order to allow the classifier
                            // threads to wakeup and notice the change in classifier.
                            state.cache.clear();
                            state.uncat_cache.clear();
                        }
                        Err(err) => {
                            had_error = true;
                            tracing::error!("Error reloading bounce classifier: {err:#}");
                        }
                    }
                }
            });
        }

        Ok(Self { tx, state })
    }

    fn check_cache(&self, response: &Response) -> Option<BounceClass> {
        let mut state = self.state.lock();
        if let Some(result) = state.cache.get_mut(response) {
            return Some(result.clone());
        }
        if let Some(result) = state.uncat_cache.get_mut(response) {
            return Some(result.clone());
        }

        None
    }

    async fn classify(&self, response: Response) -> anyhow::Result<BounceClass> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send_async(ClassifyRequest {
                response,
                tx,
                epoch: get_current_epoch(),
            })
            .await?;
        Ok(rx.await?)
    }
}

pub async fn apply_classification(record: &mut JsonLogRecord) {
    // No sense classifying receptions or deliveries as bounces, as they are not bounces!
    if matches!(record.kind, RecordType::Reception | RecordType::Delivery) {
        return;
    }

    // If you have no classifier, you pay no cost
    let Some(classifier) = CLASSIFY.get() else {
        return;
    };

    let _timer = CLASSIFY_LATENCY.start_timer();

    // Check the caches before we commit any serious resources to
    // classifying this response
    if let Some(result) = classifier.check_cache(&record.response) {
        record.bounce_classification = result;
        return;
    }

    // clone data and pass to the classifier thread pool
    if let Ok(result) = classifier.classify(record.response.clone()).await {
        record.bounce_classification = result;
    }
}
