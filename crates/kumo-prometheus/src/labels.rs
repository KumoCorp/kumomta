#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct Labels {
    labels: Vec<String>,
}

impl Labels {
    pub fn new(labels: &[&str]) -> Self {
        Self {
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn labels_ref(&self) -> Vec<&str> {
        self.labels.iter().map(|s| s.as_str()).collect()
    }
}
