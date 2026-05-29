# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

## Fixes

 * [kumo.jsonl.new_tailer](../reference/kumo.jsonl/new_tailer.md) and
   [kumo.jsonl.new_multi_tailer](../reference/kumo.jsonl/new_multi_tailer.md)
   no longer shut down on a truncated trailing record from a killed
   producer, a file that is not a valid zstd stream, or a file whose
   decompressed contents are not JSONL.  The offending file is logged and
   skipped, and unreadable files are remembered for the lifetime of the
   tailer so they are not re-attempted and cannot hide later segments
   whose names sort before them.

