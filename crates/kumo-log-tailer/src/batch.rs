use camino::Utf8PathBuf;

/// A batch of log records yielded by the tailer stream.
///
/// A batch may contain records from multiple segment files when a
/// file boundary is crossed while filling the batch.
#[derive(Debug, Clone)]
pub struct LogBatch {
    /// The lines that comprise the batch.
    lines: Vec<String>,
    /// The list of unique segment file names that were
    /// the source of data for the lines.
    file_names: Vec<Utf8PathBuf>,
    /// The indices of this vec correspond to the indices of `lines`.
    /// The elements in the vec are the indices into `file_names`
    /// of the file name from which the line was read.
    line_to_file_name: Vec<usize>,
    /// The indices of this vec correspond to the indices of `lines`.
    /// The elements in the vec are the byte offset within the
    /// decompressed stream of the start of that line.
    byte_offsets: Vec<u64>,
}

impl LogBatch {
    /// Create a new empty batch.
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            file_names: Vec::new(),
            line_to_file_name: Vec::new(),
            byte_offsets: Vec::new(),
        }
    }

    /// Add a line to the batch along with the segment it was read from
    /// and its byte offset in the decompressed stream.
    pub fn push(&mut self, line: String, segment: &Utf8PathBuf, byte_offset: u64) {
        let file_idx = match self.file_names.iter().rposition(|f| f == segment) {
            Some(idx) => idx,
            None => {
                self.file_names.push(segment.clone());
                self.file_names.len() - 1
            }
        };
        self.lines.push(line);
        self.line_to_file_name.push(file_idx);
        self.byte_offsets.push(byte_offset);
    }

    /// The number of records in this batch.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The list of unique segment file names that contributed lines
    /// to this batch.
    pub fn file_names(&self) -> &[Utf8PathBuf] {
        &self.file_names
    }

    /// Return the segment file name for the line at `index`.
    pub fn file_name_for_line(&self, index: usize) -> &Utf8PathBuf {
        &self.file_names[self.line_to_file_name[index]]
    }

    /// Return the byte offset in the decompressed stream for the line
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

impl AsRef<[String]> for LogBatch {
    fn as_ref(&self) -> &[String] {
        &self.lines
    }
}

impl TryFrom<&LogBatch> for Vec<serde_json::Value> {
    type Error = anyhow::Error;

    fn try_from(batch: &LogBatch) -> anyhow::Result<Self> {
        batch
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                serde_json::from_str(line).map_err(|err| {
                    anyhow::anyhow!(
                        "Failed to parse a line from {} (byte offset {}) \
                         as json: {err}. Is the file corrupt? You may need to move \
                         the file aside to make progress",
                        batch.file_name_for_line(i),
                        batch.byte_offset_for_line(i)
                    )
                })
            })
            .collect()
    }
}

impl TryFrom<LogBatch> for Vec<serde_json::Value> {
    type Error = anyhow::Error;

    fn try_from(batch: LogBatch) -> anyhow::Result<Self> {
        Vec::try_from(&batch)
    }
}
