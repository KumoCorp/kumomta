use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use thiserror::Error;
use zstd_safe::{DCtx, InBuffer, OutBuffer};

#[derive(Error, Debug)]
#[error("{}", zstd_safe::get_error_name(*.0))]
pub struct ZStdError(pub usize);

/// Default limit, in bytes, on how large a decompressed record may be. A record
/// that reaches it without a newline is treated as corruption. Large enough
/// that legitimate records are not expected to approach it, while still
/// catching a stream that never produces a newline.
pub const DEFAULT_MAX_LINE_SIZE: usize = 128 * 1024 * 1024;

/// A line extracted from the decompressed stream, along with its
/// byte offset in the decompressed data.
pub struct DecompressedLine {
    pub text: String,
    pub byte_offset: u64,
}

/// State for incremental zstd decompression and line extraction from a single file.
pub struct FileDecompressor {
    file: BufReader<std::fs::File>,
    context: DCtx<'static>,
    out_buffer: Vec<u8>,
    /// Steady-state size of `out_buffer`. The buffer grows past this to hold an
    /// oversized record and is shrunk back to it once the record is emitted.
    base_out_buffer: usize,
    /// Largest `out_buffer` may grow to before a record is treated as
    /// unterminated corruption rather than a legitimately large line.
    max_out_buffer: usize,
    /// Start of the next unprocessed line in `out_buffer`.
    line_start: usize,
    /// Number of valid bytes in `out_buffer`.
    out_pos: usize,
    /// Total number of lines decompressed so far.
    lines_decompressed: usize,
    /// Global line index up to which lines have been consumed or skipped.
    /// This is the value that should be used for checkpointing.
    /// Equals skip_before + number of lines actually returned to caller.
    pub lines_consumed: usize,
    /// Buffered lines that have been extracted but not yet consumed.
    pending_lines: VecDeque<DecompressedLine>,
    /// Whether we've seen EOF on the compressed input.
    saw_eof: bool,
    /// Cumulative byte offset in the decompressed stream.
    /// Tracks the position of `line_start` relative to the start
    /// of the decompressed output.
    decompressed_offset: u64,
}

impl FileDecompressor {
    /// Open a file and prepare for incremental zstd decompression.
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        Self::open_with_max_line_size(path, DEFAULT_MAX_LINE_SIZE)
    }

    /// Open a file, capping the output buffer at `max_out_buffer` bytes. A
    /// record that fills the cap without a newline is rejected as corrupt.
    pub fn open_with_max_line_size(
        path: &std::path::Path,
        max_out_buffer: usize,
    ) -> anyhow::Result<Self> {
        let file = BufReader::new(
            std::fs::File::open(path)
                .map_err(|e| anyhow::anyhow!("opening {} for read: {e}", path.display()))?,
        );
        let mut context = DCtx::create();
        context
            .init()
            .map_err(ZStdError)
            .map_err(|e| anyhow::anyhow!("initialize zstd decompression context: {e}"))?;
        context
            .load_dictionary(&[])
            .map_err(ZStdError)
            .map_err(|e| anyhow::anyhow!("load empty dictionary: {e}"))?;

        // Keep at least one byte so the growth check has a non-empty buffer to
        // fill. A misconfigured tiny cap simply rejects records as corrupt.
        let max_out_buffer = max_out_buffer.max(1);
        let base_out_buffer = DCtx::out_size().min(max_out_buffer);
        Ok(Self {
            file,
            context,
            out_buffer: vec![0u8; base_out_buffer],
            base_out_buffer,
            max_out_buffer,
            line_start: 0,
            out_pos: 0,
            lines_decompressed: 0,
            lines_consumed: 0,
            pending_lines: VecDeque::new(),
            saw_eof: false,
            decompressed_offset: 0,
        })
    }

    /// Get the next line from this file.
    ///
    /// `skip_before`: lines with index < skip_before are discarded.
    ///
    /// Returns:
    /// - `Ok(Some(line))` — a complete line was extracted.
    /// - `Ok(None)` — no more data available right now. The caller should check
    ///   if the file is done or retry later.
    pub fn next_line(&mut self, skip_before: usize) -> anyhow::Result<Option<DecompressedLine>> {
        // Return a buffered line if available
        if let Some(line) = self.pending_lines.pop_front() {
            self.lines_consumed += 1;
            return Ok(Some(line));
        }

        // If we previously saw EOF and have no buffered lines, signal EOF
        if self.saw_eof {
            return Ok(None);
        }

        // Account for skipped lines in lines_consumed
        if self.lines_consumed < skip_before {
            self.lines_consumed = skip_before;
        }

        // Read and decompress more data
        loop {
            let in_buffer = self.file.fill_buf()?;
            if in_buffer.is_empty() {
                self.saw_eof = true;
                // Return any buffered line
                if let Some(line) = self.pending_lines.pop_front() {
                    self.lines_consumed += 1;
                    return Ok(Some(line));
                }
                return Ok(None);
            }

            let mut src = InBuffer::around(in_buffer);
            let mut dest = OutBuffer::around_pos(&mut self.out_buffer, self.out_pos);

            self.context
                .decompress_stream(&mut dest, &mut src)
                .map_err(ZStdError)
                .map_err(|e| anyhow::anyhow!("zstd decompress: {e}"))?;

            let bytes_read = {
                let pos = src.pos();
                drop(src);
                pos
            };
            self.file.consume(bytes_read);
            self.out_pos = dest.pos();

            // Extract complete lines
            while let Some(idx) =
                memchr::memchr(b'\n', &self.out_buffer[self.line_start..self.out_pos])
            {
                let line_byte_offset = self.decompressed_offset;
                if self.lines_decompressed >= skip_before {
                    let this_line = &self.out_buffer[self.line_start..self.line_start + idx];
                    let line = String::from_utf8_lossy(this_line).into_owned();
                    self.pending_lines.push_back(DecompressedLine {
                        text: line,
                        byte_offset: line_byte_offset,
                    });
                }
                // Advance past the line content + newline
                let consumed = idx + 1;
                self.decompressed_offset += consumed as u64;
                self.line_start += consumed;
                self.lines_decompressed += 1;
            }

            // Compact the output buffer
            if self.line_start == self.out_pos {
                self.out_pos = 0;
                self.line_start = 0;
            } else if self.line_start > 0 {
                self.out_buffer
                    .copy_within(self.line_start..self.out_pos, 0);
                self.out_pos -= self.line_start;
                self.line_start = 0;
            }

            // Release memory borrowed to hold an oversized record (see
            // base_out_buffer) once the remaining live data fits in the
            // steady-state size again.
            if self.out_buffer.len() > self.base_out_buffer && self.out_pos <= self.base_out_buffer
            {
                self.out_buffer.truncate(self.base_out_buffer);
                self.out_buffer.shrink_to_fit();
            }

            // If we extracted any lines, return the first one
            if let Some(line) = self.pending_lines.pop_front() {
                self.lines_consumed += 1;
                return Ok(Some(line));
            }

            // A full buffer that doesn't hold any complete lines (compaction
            // always leaves line_start at 0) means the current record is larger
            // than the buffer. Grow it (up to max_out_buffer) to let the next
            // decompress call make progress; otherwise zstd stalls with "no
            // progress ... output buffer full" and the caller discards the
            // whole segment.
            if self.out_pos == self.out_buffer.len() {
                if self.out_buffer.len() >= self.max_out_buffer {
                    anyhow::bail!(
                        "record exceeds maximum line size of {} bytes",
                        self.max_out_buffer
                    );
                }
                let new_len = self
                    .out_buffer
                    .len()
                    .saturating_mul(2)
                    .min(self.max_out_buffer);
                // Reserve fallibly: max_out_buffer comes straight from operator
                // config and can be arbitrarily large. A huge cap is reported
                // as an error here (skipping this segment) instead of aborting
                // the process on a failed allocation.
                let additional = new_len - self.out_buffer.len();
                self.out_buffer.try_reserve(additional).map_err(|e| {
                    anyhow::anyhow!("allocating {new_len} bytes for the decompression buffer: {e}")
                })?;
                self.out_buffer.resize(new_len, 0);
            }

            // No complete lines yet; read more data
        }
    }

    /// Reset the EOF flag so we can try reading more data
    /// (useful when tailing a file that is still being written to).
    pub fn reset_eof(&mut self) {
        self.saw_eof = false;
    }

    /// Returns true if there is partial (incomplete line) data remaining
    /// in the output buffer.
    pub fn has_partial_data(&self) -> bool {
        self.out_pos > 0
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;
    use zstd::stream::write::Encoder;

    /// Compress the supplied lines into a zstd JSONL segment on disk, matching
    /// the framing produced by the log writer, and return the path.
    fn write_segment(lines: &[String]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut encoder = Encoder::new(file.reopen().unwrap(), 3).unwrap();
        for line in lines {
            encoder.write_all(line.as_bytes()).unwrap();
            encoder.write_all(b"\n").unwrap();
        }
        encoder.finish().unwrap();
        file
    }

    fn read_all(path: &std::path::Path) -> anyhow::Result<Vec<String>> {
        let mut decompressor = FileDecompressor::open(path)?;
        let mut lines = vec![];
        while let Some(line) = decompressor.next_line(0)? {
            lines.push(line.text);
        }
        Ok(lines)
    }

    #[test]
    fn small_records_roundtrip() {
        let input: Vec<String> = (0..1000).map(|i| format!("{{\"n\":{i}}}")).collect();
        let seg = write_segment(&input);
        let lines = read_all(seg.path()).unwrap();
        k9::assert_equal!(lines, input);
    }

    /// Verifies that a record whose decompressed size exceeds the fixed output
    /// buffer (`DCtx::out_size()`) is still recovered, along with the ordinary
    /// records that share its segment.
    #[test]
    fn record_larger_than_output_buffer() {
        let big_value = "x".repeat(DCtx::out_size() * 2);
        let input = vec![
            r#"{"n":"before"}"#.to_string(),
            format!(r#"{{"big":"{big_value}"}}"#),
            r#"{"n":"after"}"#.to_string(),
        ];
        let seg = write_segment(&input);
        let lines = read_all(seg.path()).unwrap();
        k9::assert_equal!(lines, input);
    }

    /// After an oversized record forces the buffer to grow, it shrinks back
    /// down to the steady-state size once that record has been emitted.
    #[test]
    fn buffer_shrinks_back_after_large_record() {
        let big_value = "x".repeat(DCtx::out_size() * 4);
        let input = vec![
            format!(r#"{{"big":"{big_value}"}}"#),
            r#"{"n":"after"}"#.to_string(),
        ];
        let seg = write_segment(&input);
        let mut decompressor = FileDecompressor::open(seg.path()).unwrap();

        // Recovering the oversized record forces the buffer past its base size
        // (proven by record_larger_than_output_buffer). By the time it is
        // emitted the buffer has already been reclaimed.
        let first = decompressor.next_line(0).unwrap().unwrap();
        k9::assert_equal!(first.text, input[0]);
        k9::assert_equal!(decompressor.out_buffer.len(), decompressor.base_out_buffer);

        let second = decompressor.next_line(0).unwrap().unwrap();
        k9::assert_equal!(second.text, input[1]);
        k9::assert_equal!(decompressor.out_buffer.len(), decompressor.base_out_buffer);
    }

    /// With the output buffer capped, a record that never terminates within the
    /// cap is reported as an error.
    #[test]
    fn record_exceeding_cap_is_rejected() {
        let cap = 64 * 1024;
        let input = vec![format!(r#"{{"big":"{}"}}"#, "x".repeat(cap * 2))];
        let seg = write_segment(&input);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        let err = match decompressor.next_line(0) {
            Err(err) => err,
            Ok(_) => panic!("expected an error for the oversized record"),
        };
        k9::assert_equal!(
            err.to_string(),
            format!("record exceeds maximum line size of {cap} bytes")
        );
    }

    /// Reads a record one byte below the cap: its content plus the newline
    /// separator fit within the buffer.
    #[test]
    fn record_just_below_cap_is_read() {
        let cap = 64 * 1024;
        let content = "x".repeat(cap - 1);
        let seg = write_segment(&[content.clone()]);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        let line = decompressor.next_line(0).unwrap().unwrap();
        k9::assert_equal!(line.text, content);
    }

    /// Rejects a record whose content is exactly `cap` bytes, one byte too long
    /// for its trailing newline to also fit in the buffer.
    #[test]
    fn record_at_cap_is_rejected() {
        let cap = 64 * 1024;
        let content = "y".repeat(cap);
        let seg = write_segment(&[content]);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        match decompressor.next_line(0) {
            Err(_) => {}
            Ok(other) => panic!(
                "expected an error for a record whose content equals the cap, got {:?}",
                other.map(|l| l.text.len())
            ),
        }
    }
}
