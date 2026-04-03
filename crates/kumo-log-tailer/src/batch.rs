use camino::Utf8PathBuf;

/// A batch of log records yielded by the tailer stream.
///
/// Each record is a parsed JSON value.  A batch may contain records
/// from multiple segment files when a file boundary is crossed while
/// filling the batch.
#[derive(Debug, Clone)]
pub struct LogBatch {
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
}

impl LogBatch {
    /// Create a new empty batch.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            file_names: Vec::new(),
            line_to_file_name: Vec::new(),
            byte_offsets: Vec::new(),
        }
    }

    /// Parse `line` as JSON and add it to the batch.
    ///
    /// Returns an error with context (segment file name and byte offset)
    /// if the line is not valid JSON.
    pub fn push(
        &mut self,
        line: &str,
        segment: &Utf8PathBuf,
        byte_offset: u64,
    ) -> anyhow::Result<()> {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse a line from {segment} (byte offset {byte_offset}) \
                 as json: {err}. Is the file corrupt? You may need to move \
                 the file aside to make progress"
            )
        })?;
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
