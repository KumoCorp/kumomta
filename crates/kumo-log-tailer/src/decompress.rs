use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use thiserror::Error;
use zstd_safe::{DCtx, InBuffer, OutBuffer};

#[derive(Error, Debug)]
#[error("{}", zstd_safe::get_error_name(*.0))]
pub struct ZStdError(pub usize);

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

        Ok(Self {
            file,
            context,
            out_buffer: vec![0u8; DCtx::out_size()],
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

            // If we extracted any lines, return the first one
            if let Some(line) = self.pending_lines.pop_front() {
                self.lines_consumed += 1;
                return Ok(Some(line));
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
