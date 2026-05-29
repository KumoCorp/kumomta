# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

## Fixes

 * The `feedback_report.original_message` field, and the values in the
   associated `extensions` map, in `Feedback` log records produced for
   incoming ARF reports were being serialized as a JSON array of byte
   values rather than the string shape documented in the
   [log_record](../reference/log_record.md) reference. They are now
   emitted as a JSON string when the underlying bytes are valid UTF-8,
   falling back to a byte array only for non-UTF-8 content. #529

 * [kumo.jsonl.new_tailer](../reference/kumo.jsonl/new_tailer.md) and
   [kumo.jsonl.new_multi_tailer](../reference/kumo.jsonl/new_multi_tailer.md)
   no longer shut down on a truncated trailing record from a killed
   producer, a file that is not a valid zstd stream, or a file whose
   decompressed contents are not JSONL.  The offending file is logged and
   skipped, and unreadable files are remembered for the lifetime of the
   tailer so they are not re-attempted and cannot hide later segments
   whose names sort before them.

