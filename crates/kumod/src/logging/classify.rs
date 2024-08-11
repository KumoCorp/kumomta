use anyhow::anyhow;
use bounce_classify::{BounceClassifier, BounceClassifierBuilder};
use kumo_log_types::JsonLogRecord;
use once_cell::sync::OnceCell;
use serde::Deserialize;

static CLASSIFY: OnceCell<BounceClassifier> = OnceCell::new();

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ClassifierParams {
    pub files: Vec<String>,
}

impl ClassifierParams {
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
            .set(classifier)
            .map_err(|_| anyhow::anyhow!("classifier already initialized"))?;

        Ok(())
    }
}

pub fn apply_classification(record: &mut JsonLogRecord) {
    if let Some(classifier) = CLASSIFY.get() {
        record.bounce_classification = classifier.classify_response(&record.response);
    }
}
