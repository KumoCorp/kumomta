use anyhow::anyhow;
use bounce_classify::{
    BounceClass, BounceClassifier, BounceClassifierBuilder, PreDefinedBounceClass,
};
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

    pub fn register(&self) -> anyhow::Result<()> {
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

        let classifier = builder.build().map_err(|err| anyhow!("{err}"))?;

        CLASSIFY
            .set(ClassifierWrapper::new(classifier, self)?)
            .map_err(|_| anyhow::anyhow!("classifier already initialized"))?;

        Ok(())
    }
}

struct ClassifyRequest {
    response: Response,
    tx: oneshot::Sender<BounceClass>,
}

/// We maintain two caches; one for uncategorized results and the
/// other for categorized results. The rationale is that there
/// could be a large set of responses that do not have a categorization
/// and we don't want those to thrash and out-compete the actual
/// classifications and render the whole class completely ineffective.
struct State {
    cache: LruCache<Response, BounceClass>,
    uncat_cache: LruCache<Response, BounceClass>,
}

impl State {
    fn insert(&mut self, response: Response, result: BounceClass) {
        let cache = match &result {
            BounceClass::PreDefined(PreDefinedBounceClass::Uncategorized) => &mut self.uncat_cache,
            _ => &mut self.cache,
        };

        cache.insert(response.clone(), result.clone());
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

        let state = Arc::new(Mutex::new(State {
            cache: LruCache::new(params.cache_size),
            uncat_cache: LruCache::new(params.uncategorized_cache_size),
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
                    while let Ok(ClassifyRequest { response, tx }) = rx.recv() {
                        let result = classifier.classify_response(&response);
                        state.lock().insert(response.clone(), result.clone());
                        if tx.send(result).is_err() {
                            break;
                        }
                    }
                })?;
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
        self.tx.send_async(ClassifyRequest { response, tx }).await?;
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
