# Unreleased Changes in The Mainline

## Breaking Changes

* [kumo.configure_local_logs](../reference/kumo/configure_local_logs/index.md)
  now enforces a
  [max_record_size](../reference/kumo/configure_local_logs/max_record_size.md)
  (default 128 MiB): a record whose serialized content reaches the limit is
  dropped rather than written, counted by the `log_record_dropped_too_large`
  metric and a rate-limited error log. Previously every record was written
  regardless of size. The default matches the reader's `max_line_size` (also
  128 MiB), keeping every written record readable. Before this version the
  reader was fixed at roughly 128 KiB and could not be adjusted, so a record
  larger than that was silently unreadable. Raise `max_record_size` to keep
  larger records.
* [kumo.jsonl.new_writer](../reference/kumo.jsonl/new_writer.md)'s `write_line`
  and `write_record` now raise an error for a record that reaches
  `max_record_size` (default 128 MiB) instead of writing it. The default
  matches the reader's `max_line_size`, keeping every written record readable;
  before this version the reader was fixed at roughly 128 KiB and could not be
  raised.

## Other Changes and Enhancements

## Fixes

 * [kumo.jsonl.new_tailer](../reference/kumo.jsonl/new_tailer.md) no longer
   discards the rest of a log segment when it meets a record larger than the
   fixed zstd output block. Previously the decompression buffer was fixed at
   that block size (roughly 128 KiB) with no way to adjust it, so any larger
   record stalled decompression and the tailer skipped the rest of the segment.
   The buffer now grows on demand to read records up to the new configurable
   `max_line_size` (default 128 MiB). A record larger than that limit is dropped,
   and the rest of the segment is still read.
 * Fixed a panic when folding a header line that contained a single run of
   text longer than the wrap limit with multi-byte (non-ASCII) characters,
   such as an internationalized address or a long UTF-8 word passed to
   `string.wrap()`. The over-long run was split byte by byte, which could
   cut a UTF-8 sequence in half and panic when the result was validated as
   UTF-8. It is now split on character boundaries.
