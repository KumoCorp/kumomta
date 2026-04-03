use camino::Utf8PathBuf;

/// Metadata about a single line within a [`LogBatch`].
#[derive(Debug, Clone)]
pub struct LineInfo {
    /// The byte offset of this line within the decompressed stream
    /// of the segment file.
    pub byte_offset: u64,
}

/// A batch of log records yielded by the tailer stream.
///
/// Contains the raw line strings along with metadata about which
/// segment file they came from and the byte offset of each line.
#[derive(Debug, Clone)]
pub struct LogBatch {
    /// The path of the segment file these records were read from.
    segment: Utf8PathBuf,
    /// The raw log lines.
    lines: Vec<String>,
    /// Per-line metadata (byte offset, etc.), parallel to `lines`.
    line_info: Vec<LineInfo>,
}

impl LogBatch {
    /// Create a new empty batch for the given segment file.
    pub fn new(segment: Utf8PathBuf) -> Self {
        Self {
            segment,
            lines: Vec::new(),
            line_info: Vec::new(),
        }
    }

    /// Add a line to the batch along with its byte offset in the
    /// decompressed stream.
    pub fn push(&mut self, line: String, byte_offset: u64) {
        self.lines.push(line);
        self.line_info.push(LineInfo { byte_offset });
    }

    /// The number of records in this batch.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The path of the segment file these records were read from.
    pub fn segment(&self) -> &Utf8PathBuf {
        &self.segment
    }

    /// Per-line metadata, parallel to the lines returned by
    /// [`AsRef<[String]>`].
    pub fn line_info(&self) -> &[LineInfo] {
        &self.line_info
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
        let segment = batch.segment();
        batch
            .lines
            .iter()
            .zip(batch.line_info.iter())
            .map(|(line, info)| {
                serde_json::from_str(line).map_err(|err| {
                    anyhow::anyhow!(
                        "Failed to parse a line from {segment} (byte offset {}) \
                         as json: {err}. Is the file corrupt? You may need to move \
                         the file aside to make progress",
                        info.byte_offset
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
