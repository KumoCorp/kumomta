use camino::Utf8PathBuf;

/// Callback that writes a checkpoint when invoked.
type CommitFn = Box<dyn FnOnce() -> anyhow::Result<()> + Send>;

/// A batch of log records yielded by the tailer stream.
///
/// Each record is a parsed JSON value.  A batch may contain records
/// from multiple segment files when a file boundary is crossed while
/// filling the batch.
///
/// Call [`LogBatch::commit`] after processing the batch to advance
/// the checkpoint.  If `commit` is not called the checkpoint remains
/// at its prior position, so the records in this batch will be
/// re-read on the next run.
pub struct LogBatch {
    /// The name of the consumer this batch belongs to.
    consumer_name: String,
    /// The parsed JSON records that comprise the batch.
    records: Vec<serde_json::Value>,
    /// The list of unique segment file names that were
    /// the source of data for the records.
    file_names: Vec<Utf8PathBuf>,
    /// The indices of this vec correspond to the indices of `records`.
    /// The elements in the vec are the indices into `file_names`
    /// of the file name from which the record was read.
    line_to_file_name: Vec<usize>,
    /// The indices of this vec correspond to the indices of `records`.
    /// The elements in the vec are the byte offset within the
    /// decompressed stream of the start of that record.
    byte_offsets: Vec<u64>,
    /// Callback that writes the checkpoint for this batch.
    /// Set by the stream before yielding.  Consumed by `commit()`.
    commit_fn: Option<CommitFn>,
}

impl LogBatch {
    /// Create a new empty batch with a default consumer name.
    pub fn new() -> Self {
        Self::with_consumer_name(String::new())
    }

    /// Create a new empty batch for the named consumer.
    pub fn with_consumer_name(name: String) -> Self {
        Self {
            consumer_name: name,
            records: Vec::new(),
            file_names: Vec::new(),
            line_to_file_name: Vec::new(),
            byte_offsets: Vec::new(),
            commit_fn: None,
        }
    }

    /// Add a pre-parsed JSON value to the batch.
    pub(crate) fn push_value(
        &mut self,
        value: serde_json::Value,
        segment: &Utf8PathBuf,
        byte_offset: u64,
    ) {
        let file_idx = match self.file_names.iter().rposition(|f| f == segment) {
            Some(idx) => idx,
            None => {
                self.file_names.push(segment.clone());
                self.file_names.len() - 1
            }
        };
        self.records.push(value);
        self.line_to_file_name.push(file_idx);
        self.byte_offsets.push(byte_offset);
    }

    /// Set the commit callback.  Called by the stream before yielding.
    pub(crate) fn set_commit_fn(&mut self, f: CommitFn) {
        self.commit_fn = Some(f);
    }

    /// The name of the consumer this batch belongs to.
    pub fn consumer_name(&self) -> &str {
        &self.consumer_name
    }

    /// Advance the checkpoint to the end of this batch.
    ///
    /// This confirms that the caller has successfully processed
    /// the batch.  If checkpointing is not enabled, or if this
    /// batch has already been committed, this is a no-op.
    pub fn commit(&mut self) -> anyhow::Result<()> {
        if let Some(f) = self.commit_fn.take() {
            f()?;
        }
        Ok(())
    }

    /// The number of records in this batch.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The parsed JSON records.
    pub fn records(&self) -> &[serde_json::Value] {
        &self.records
    }

    /// The list of unique segment file names that contributed records
    /// to this batch.
    pub fn file_names(&self) -> &[Utf8PathBuf] {
        &self.file_names
    }

    /// Return the segment file name for the record at `index`.
    pub fn file_name_for_line(&self, index: usize) -> &Utf8PathBuf {
        &self.file_names[self.line_to_file_name[index]]
    }

    /// Return the byte offset in the decompressed stream for the record
    /// at `index`.
    pub fn byte_offset_for_line(&self, index: usize) -> u64 {
        self.byte_offsets[index]
    }
}

impl Default for LogBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<[serde_json::Value]> for LogBatch {
    fn as_ref(&self) -> &[serde_json::Value] {
        &self.records
    }
}
