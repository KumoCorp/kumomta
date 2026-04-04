use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use std::io::Write;
use std::time::Duration;
use zstd::stream::write::Encoder;

/// Configuration for constructing a [`LogWriter`].
pub struct LogWriterConfig {
    /// Directory where log segment files will be created.
    pub log_dir: Utf8PathBuf,
    /// Maximum number of uncompressed bytes written per segment
    /// before rolling to a new file.
    pub max_file_size: u64,
    /// Zstd compression level.
    pub compression_level: i32,
    /// If set, the segment will be closed after this duration
    /// even if max_file_size has not been reached.
    pub max_segment_duration: Option<Duration>,
    /// Optional suffix appended to segment file names.
    pub suffix: Option<String>,
    /// Timezone used when computing the segment file name.
    /// Defaults to UTC.
    pub tz: Option<Tz>,
}

impl LogWriterConfig {
    pub fn new(log_dir: Utf8PathBuf) -> Self {
        Self {
            log_dir,
            max_file_size: 128 * 1024 * 1024,
            compression_level: 3,
            max_segment_duration: None,
            suffix: None,
            tz: None,
        }
    }

    pub fn max_file_size(mut self, size: u64) -> Self {
        self.max_file_size = size;
        self
    }

    pub fn compression_level(mut self, level: i32) -> Self {
        self.compression_level = level;
        self
    }

    pub fn max_segment_duration(mut self, duration: Duration) -> Self {
        self.max_segment_duration = Some(duration);
        self
    }

    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    pub fn tz(mut self, tz: Tz) -> Self {
        self.tz = Some(tz);
        self
    }

    /// Build the [`LogWriter`].
    pub fn build(self) -> LogWriter {
        LogWriter {
            log_dir: self.log_dir,
            max_file_size: self.max_file_size,
            compression_level: self.compression_level,
            max_segment_duration: self.max_segment_duration,
            suffix: self.suffix,
            tz: self.tz,
            current: None,
        }
    }
}

/// State for the currently open segment file.
struct OpenSegment {
    encoder: Encoder<'static, std::fs::File>,
    path: Utf8PathBuf,
    bytes_written: u64,
    opened_at: std::time::Instant,
}

/// A log writer that produces zstd-compressed JSONL segment files
/// in a directory, compatible with [`LogTailer`](crate::LogTailer).
pub struct LogWriter {
    log_dir: Utf8PathBuf,
    max_file_size: u64,
    compression_level: i32,
    max_segment_duration: Option<Duration>,
    suffix: Option<String>,
    tz: Option<Tz>,
    current: Option<OpenSegment>,
}

impl LogWriter {
    /// Write a JSONL line (record) to the current segment.
    ///
    /// If no segment is open, one will be created.  After writing,
    /// if the segment has exceeded `max_file_size` or
    /// `max_segment_duration` it will be closed and a new segment
    /// will be opened on the next write.
    pub fn write_line(&mut self, line: &str) -> anyhow::Result<()> {
        if self.current.is_none() {
            self.open_segment()?;
        }

        let seg = self.current.as_mut().expect("just opened");

        seg.encoder.write_all(line.as_bytes())?;
        seg.bytes_written += line.len() as u64;

        // Ensure the record ends with a newline
        if !line.ends_with('\n') {
            seg.encoder.write_all(b"\n")?;
            seg.bytes_written += 1;
        }

        // Check if we need to roll to a new segment
        if self.should_roll() {
            self.close_segment()?;
        }

        Ok(())
    }

    /// Serialize `value` to JSON and write it as a JSONL line.
    pub fn write_value<S: serde::Serialize>(&mut self, value: &S) -> anyhow::Result<()> {
        let json = serde_json::to_string(value)?;
        self.write_line(&json)
    }

    /// Flush and close the current segment if it has exceeded
    /// `max_segment_duration`.  This is a no-op if no segment is
    /// open or if the duration has not been exceeded.
    pub fn maintain(&mut self) -> anyhow::Result<()> {
        if self.current.is_some() && self.duration_exceeded() {
            self.close_segment()?;
        }
        Ok(())
    }

    /// Flush and close the current segment, regardless of whether
    /// it has exceeded any configured constraints.
    pub fn close(&mut self) -> anyhow::Result<()> {
        if self.current.is_some() {
            self.close_segment()?;
        }
        Ok(())
    }

    /// Finish the zstd stream so the data is readable, but do NOT
    /// mark the file as done (readonly).  This is useful for tests
    /// that need to simulate an in-progress segment.
    pub fn flush_without_marking_done(&mut self) -> anyhow::Result<()> {
        if let Some(seg) = self.current.take() {
            seg.encoder.finish()?;
        }
        Ok(())
    }

    fn should_roll(&self) -> bool {
        let Some(seg) = &self.current else {
            return false;
        };
        if seg.bytes_written >= self.max_file_size {
            return true;
        }
        self.duration_exceeded()
    }

    fn duration_exceeded(&self) -> bool {
        let Some(seg) = &self.current else {
            return false;
        };
        if let Some(max_dur) = self.max_segment_duration {
            if seg.opened_at.elapsed() >= max_dur {
                return true;
            }
        }
        false
    }

    fn open_segment(&mut self) -> anyhow::Result<()> {
        let now: DateTime<Utc> = Utc::now();
        let mut base_name = match &self.tz {
            Some(tz) => now.with_timezone(tz).format("%Y%m%d-%H%M%S%.f").to_string(),
            None => now.format("%Y%m%d-%H%M%S%.f").to_string(),
        };
        if let Some(suffix) = &self.suffix {
            base_name.push_str(suffix);
        }
        let path = self.log_dir.join(base_name);

        std::fs::create_dir_all(&self.log_dir)?;

        let file = std::fs::File::create(path.as_std_path())?;
        let encoder = Encoder::new(file, self.compression_level)?;

        self.current = Some(OpenSegment {
            encoder,
            path,
            bytes_written: 0,
            opened_at: std::time::Instant::now(),
        });

        Ok(())
    }

    fn close_segment(&mut self) -> anyhow::Result<()> {
        if let Some(seg) = self.current.take() {
            // Finish the zstd stream (flushes and writes the end frame)
            seg.encoder.finish()?;
            // Mark the file as done (readonly) to signal to the tailer
            // that this segment is complete
            mark_segment_done(&seg.path)?;
        }
        Ok(())
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        // Best-effort close on drop
        let _ = self.close();
    }
}

/// Mark a segment file as done by removing write permissions.
fn mark_segment_done(path: &Utf8PathBuf) -> std::io::Result<()> {
    let meta = std::fs::metadata(path.as_std_path())?;
    let mut perms = meta.permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(path.as_std_path(), perms)
}
