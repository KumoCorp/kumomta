use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use thiserror::Error;
use zstd_safe::{DCtx, InBuffer, OutBuffer};

#[derive(Error, Debug)]
#[error("{}", zstd_safe::get_error_name(*.0))]
pub struct ZStdError(pub usize);

/// Default limit, in bytes, on how large a decompressed record may be. A record
/// that reaches it without a newline is discarded and the following records are
/// still read. Large enough that legitimate records are not expected to
/// approach it, while still bounding how much is buffered for one line.
pub const DEFAULT_MAX_LINE_SIZE: usize = 128 * 1024 * 1024;

/// A line extracted from the decompressed stream, along with its
/// byte offset in the decompressed data.
pub struct DecompressedLine {
    pub text: String,
    pub byte_offset: u64,
}

impl std::fmt::Debug for DecompressedLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A line may be as large as the configured maximum (128 MiB by
        // default). Truncate to a leading fragment to keep debug output, and
        // anything that logs it, bounded regardless of line length.
        const MAX: usize = 64;
        let mut dbg = f.debug_struct("DecompressedLine");
        dbg.field("byte_offset", &self.byte_offset)
            .field("len", &self.text.len());
        if self.text.len() <= MAX {
            dbg.field("text", &self.text);
        } else {
            dbg.field(
                "text_prefix",
                &&self.text[..self.text.floor_char_boundary(MAX)],
            );
        }
        dbg.finish()
    }
}

/// The outcome of a call to [`FileDecompressor::next_line`].
#[derive(Debug)]
pub enum NextLine {
    /// A complete line was extracted.
    Line(DecompressedLine),
    /// A record longer than the configured maximum was discarded and the
    /// stream continues with the following record. Reports where the discarded
    /// record began in the decompressed stream and how many bytes were dropped
    /// (excluding the terminating newline).
    Skipped { byte_offset: u64, bytes: u64 },
    /// No line is available right now. The caller should check whether the file
    /// is done or retry later.
    None,
}

/// Bookkeeping for a record being discarded because it exceeds the maximum line
/// size. Scanning continues across decompress calls until the record's
/// terminating newline is found. Discarding only consumes bytes from the
/// output buffer; the zstd stream itself is never put in an error state, so
/// the segment's later records decode normally once the newline is reached.
struct Skip {
    /// Byte offset in the decompressed stream where the discarded record began.
    byte_offset: u64,
    /// Bytes of the record discarded so far, excluding the terminating newline.
    discarded: u64,
    /// Whether the record is at or after `skip_before` and should be reported
    /// to the caller. A record before the checkpoint was consumed on an earlier
    /// run and is dropped silently.
    surface: bool,
}

/// State for incremental zstd decompression and line extraction from a single file.
pub struct FileDecompressor {
    file: BufReader<std::fs::File>,
    context: DCtx<'static>,
    out_buffer: Vec<u8>,
    /// Steady-state size of `out_buffer`. The buffer grows past this to hold an
    /// oversized record and is shrunk back to it once the record is emitted.
    base_out_buffer: usize,
    /// The maximum size `out_buffer` may grow to. A record that fills it
    /// without a newline is discarded (reported as `NextLine::Skipped`) rather
    /// than buffered further.
    max_out_buffer: usize,
    /// Start of the next unprocessed line in `out_buffer`.
    line_start: usize,
    /// Number of valid bytes in `out_buffer`.
    out_pos: usize,
    /// When discarding a record that exceeds `max_out_buffer`, the state of
    /// that in-progress skip. `None` during normal line extraction.
    skipping: Option<Skip>,
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
    /// record that fills the cap without a newline is discarded and reading
    /// continues with the next record (see [`NextLine::Skipped`]).
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
        // fill. A misconfigured tiny cap simply discards records as oversized.
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
            skipping: None,
            lines_decompressed: 0,
            lines_consumed: 0,
            pending_lines: VecDeque::new(),
            saw_eof: false,
            decompressed_offset: 0,
        })
    }

    /// Returns the next line from this file.
    ///
    /// `skip_before`: lines with index < skip_before are discarded.
    ///
    /// Returns:
    /// - `Ok(NextLine::Line(line))` -- a complete line was extracted.
    /// - `Ok(NextLine::Skipped { .. })` -- a record exceeding the maximum line
    ///   size was discarded. Reading continues with the next record.
    /// - `Ok(NextLine::None)` -- there isn't any more data available right now.
    ///   The caller should check if the file is done or retry later.
    pub fn next_line(&mut self, skip_before: usize) -> anyhow::Result<NextLine> {
        // Return a buffered line if available
        if let Some(line) = self.pending_lines.pop_front() {
            self.lines_consumed += 1;
            return Ok(NextLine::Line(line));
        }

        // If we previously saw EOF and have no buffered lines, signal EOF
        if self.saw_eof {
            return Ok(NextLine::None);
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
                // A skip in progress is left intact. If the file is still
                // being written, the caller resets EOF and we resume discarding
                // when more data arrives. If the file is done, has_partial_data
                // returns true while a skip is in progress, which is how the
                // caller recognizes that the trailing partial record should be
                // dropped.
                if let Some(line) = self.pending_lines.pop_front() {
                    self.lines_consumed += 1;
                    return Ok(NextLine::Line(line));
                }
                return Ok(NextLine::None);
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

            // Set when this iteration finishes discarding an oversized record
            // that should be reported. Extraction below queues any lines found
            // after the skip into pending_lines first. The code further down
            // deliberately checks for a pending just_skipped report ahead of
            // pending_lines, reversing the detection order: the skip is
            // reported to the caller before the lines that follow it, even
            // though those lines were extracted first.
            let mut just_skipped: Option<(u64, u64)> = None;

            // While discarding an oversized record, consume its bytes up to and
            // including its terminating newline before resuming extraction.
            if self.skipping.is_some() {
                match memchr::memchr(b'\n', &self.out_buffer[..self.out_pos]) {
                    None => {
                        // The whole buffer is more of the record. Drop it and
                        // read more. `self.skipping.is_some()` guards this
                        // whole match, and the code that enlarges `out_buffer`
                        // runs only when `self.skipping` is `None`. The
                        // conditions cannot both hold, so entering this branch
                        // guarantees the resize code does not run this
                        // iteration. `out_buffer` holds at `base_out_buffer`
                        // bytes for every iteration of a skip, whatever the
                        // size of the discarded record.
                        let skip = self.skipping.as_mut().expect("checked skipping");
                        skip.discarded += self.out_pos as u64;
                        self.decompressed_offset += self.out_pos as u64;
                        self.out_pos = 0;
                        self.line_start = 0;
                        continue;
                    }
                    Some(idx) => {
                        // The record ends at the newline. Discard through it
                        // and resume normal extraction on whatever follows.
                        let mut skip = self.skipping.take().expect("checked skipping");
                        skip.discarded += idx as u64;
                        let consumed = idx + 1;
                        self.decompressed_offset += consumed as u64;
                        self.lines_decompressed += 1;
                        self.out_buffer.copy_within(consumed..self.out_pos, 0);
                        self.out_pos -= consumed;
                        self.line_start = 0;
                        if skip.surface {
                            just_skipped = Some((skip.byte_offset, skip.discarded));
                        }
                        // Fall through to extract the remainder. A surfaced
                        // skip is returned below, ahead of those lines.
                    }
                }
            }

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

            // Report a just-finished oversized-record skip ahead of any lines
            // that followed it in the same buffer (already queued above).
            if let Some((byte_offset, bytes)) = just_skipped {
                self.lines_consumed += 1;
                return Ok(NextLine::Skipped { byte_offset, bytes });
            }

            // If we extracted any lines, return the first one
            if let Some(line) = self.pending_lines.pop_front() {
                self.lines_consumed += 1;
                return Ok(NextLine::Line(line));
            }

            // A full buffer that doesn't hold any complete lines (compaction
            // always leaves line_start at 0) means the current record is larger
            // than the buffer. Grow it (up to max_out_buffer) to let the next
            // decompress call make progress; otherwise zstd stalls with "no
            // progress ... output buffer full".
            if self.out_pos == self.out_buffer.len() {
                if self.out_buffer.len() >= self.max_out_buffer {
                    // If a record exceeds the buffer cap, we discard just that
                    // record rather than failing the whole segment, since the
                    // zstd stream is intact (this is our own cap, not a decode
                    // error). We scan forward for its terminating newline and
                    // resume with the next record. The current buffer is
                    // dropped and the buffer shrinks back to base, since while
                    // skipping we only need room to scan.
                    let surface = self.lines_decompressed >= skip_before;
                    self.skipping = Some(Skip {
                        byte_offset: self.decompressed_offset,
                        discarded: self.out_pos as u64,
                        surface,
                    });
                    self.decompressed_offset += self.out_pos as u64;
                    self.out_pos = 0;
                    self.line_start = 0;
                    if self.out_buffer.len() > self.base_out_buffer {
                        self.out_buffer.truncate(self.base_out_buffer);
                        self.out_buffer.shrink_to_fit();
                    }
                    continue;
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

    /// Returns true if there is partial (incomplete line) data remaining: either
    /// buffered bytes with no terminating newline yet, or a record still being
    /// discarded for exceeding the maximum line size. When a done file ends in
    /// this state the trailing record is incomplete and is dropped.
    pub fn has_partial_data(&self) -> bool {
        self.out_pos > 0 || self.skipping.is_some()
    }

    /// Returns true if the trailing partial data is a record being discarded
    /// for exceeding the maximum line size, as opposed to an unterminated line
    /// left behind by a writer that exited without flushing.
    pub fn is_discarding_oversized_record(&self) -> bool {
        self.skipping.is_some()
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
        loop {
            match decompressor.next_line(0)? {
                NextLine::Line(line) => lines.push(line.text),
                NextLine::Skipped { .. } => panic!("unexpected skip"),
                NextLine::None => break,
            }
        }
        Ok(lines)
    }

    fn expect_line(decompressor: &mut FileDecompressor) -> DecompressedLine {
        match decompressor.next_line(0).unwrap() {
            NextLine::Line(line) => line,
            other => panic!("expected a line, got {other:?}"),
        }
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
        let first = expect_line(&mut decompressor);
        k9::assert_equal!(first.text, input[0]);
        k9::assert_equal!(decompressor.out_buffer.len(), decompressor.base_out_buffer);

        let second = expect_line(&mut decompressor);
        k9::assert_equal!(second.text, input[1]);
        k9::assert_equal!(decompressor.out_buffer.len(), decompressor.base_out_buffer);
    }

    /// Discards a record exceeding the cap and reports it as skipped, with its
    /// start offset and byte count, rather than failing the whole segment.
    #[test]
    fn record_exceeding_cap_is_skipped() {
        let cap = 64 * 1024;
        let content = "x".repeat(cap * 2);
        let seg = write_segment(&[content.clone()]);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        match decompressor.next_line(0).unwrap() {
            NextLine::Skipped { byte_offset, bytes } => {
                k9::assert_equal!(byte_offset, 0);
                k9::assert_equal!(bytes, content.len() as u64);
            }
            other => panic!("expected the oversized record to be skipped, got {other:?}"),
        }
        assert!(matches!(decompressor.next_line(0).unwrap(), NextLine::None));
    }

    /// Reads a record one byte below the cap: its content plus the newline
    /// separator fit within the buffer.
    #[test]
    fn record_just_below_cap_is_read() {
        let cap = 64 * 1024;
        let content = "x".repeat(cap - 1);
        let seg = write_segment(&[content.clone()]);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        k9::assert_equal!(expect_line(&mut decompressor).text, content);
    }

    /// Skips a record whose content is exactly `cap` bytes: one byte too long
    /// for its trailing newline to also fit in the buffer.
    #[test]
    fn record_at_cap_is_skipped() {
        let cap = 64 * 1024;
        let content = "y".repeat(cap);
        let seg = write_segment(&[content]);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        assert!(matches!(
            decompressor.next_line(0).unwrap(),
            NextLine::Skipped { .. }
        ));
    }

    /// Discards an oversized record between two ordinary records while the
    /// records around it are still read, and the checkpoint line count keeps
    /// advancing across the skip.
    #[test]
    fn oversized_record_skipped_rest_recovered() {
        let cap = 64 * 1024;
        let input = vec![
            "before".to_string(),
            "y".repeat(cap * 2),
            "after".to_string(),
        ];
        let seg = write_segment(&input);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();

        let before = expect_line(&mut decompressor);
        k9::assert_equal!(before.text, "before".to_string());

        match decompressor.next_line(0).unwrap() {
            NextLine::Skipped { bytes, .. } => {
                k9::assert_equal!(bytes, (cap * 2) as u64);
            }
            other => panic!("expected the oversized record to be skipped, got {other:?}"),
        }

        let after = expect_line(&mut decompressor);
        k9::assert_equal!(after.text, "after".to_string());
        // before + skipped record + after = three lines consumed.
        k9::assert_equal!(decompressor.lines_consumed, 3);

        assert!(matches!(decompressor.next_line(0).unwrap(), NextLine::None));
    }

    /// A record that exceeds the cap but sits before `skip_before` (already
    /// consumed on an earlier run) is dropped silently. Decompression always
    /// restarts from the head of a segment. This keeps an oversized record
    /// before the checkpoint from being reported again on every restart.
    #[test]
    fn oversized_record_before_checkpoint_dropped_silently() {
        let cap = 64 * 1024;
        let input = vec!["y".repeat(cap * 2), "after".to_string()];
        let seg = write_segment(&input);
        let mut decompressor = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();

        // skip_before = 1 marks the oversized first record as already consumed;
        // it must not surface as a skip, and the next call yields "after".
        match decompressor.next_line(1).unwrap() {
            NextLine::Line(line) => {
                k9::assert_equal!(line.text, "after".to_string());
            }
            other => panic!("expected the record after the skipped one, got {other:?}"),
        }
        assert!(matches!(decompressor.next_line(1).unwrap(), NextLine::None));
    }

    /// Verifies that a skip reaching EOF before the newline of the record
    /// arrives (the record is still being written) is preserved: after more
    /// data is appended and `reset_eof` is called, the skip completes with the
    /// full byte count and the following record is read with its correct
    /// offset, rather than the appended suffix being mistaken for a new record.
    #[test]
    fn skip_resumes_across_eof_when_tailing() {
        let cap = 64 * 1024;
        let content = "y".repeat(cap * 2);

        // Compress incrementally: first the content of the oversized record
        // with no newline yet (flushed to decode on its own), then its newline
        // and a following record.
        let mut encoder = Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(content.as_bytes()).unwrap();
        encoder.flush().unwrap();
        let prefix_len = encoder.get_ref().len();
        encoder.write_all(b"\nafter\n").unwrap();
        let all = encoder.finish().unwrap();
        let (prefix, suffix) = all.split_at(prefix_len);

        let seg = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(seg.path(), prefix).unwrap();

        let mut d = FileDecompressor::open_with_max_line_size(seg.path(), cap).unwrap();
        // The record is missing its terminating newline: reading scans for one
        // and finds none before the input runs out, then stops at EOF mid-skip.
        assert!(matches!(d.next_line(0).unwrap(), NextLine::None));
        assert!(d.has_partial_data());

        // The rest of the record and a following record arrive.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(seg.path())
            .unwrap();
        f.write_all(suffix).unwrap();
        f.flush().unwrap();
        drop(f);
        d.reset_eof();

        match d.next_line(0).unwrap() {
            NextLine::Skipped { byte_offset, bytes } => {
                k9::assert_equal!(byte_offset, 0);
                k9::assert_equal!(bytes, content.len() as u64);
            }
            other => panic!("expected the oversized record to be skipped, got {other:?}"),
        }
        let after = expect_line(&mut d);
        k9::assert_equal!(after.text, "after".to_string());
        k9::assert_equal!(after.byte_offset, content.len() as u64 + 1);
        k9::assert_equal!(d.lines_consumed, 2);
        assert!(matches!(d.next_line(0).unwrap(), NextLine::None));
    }
}
