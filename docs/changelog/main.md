# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * The `shaping.lua` helper's `setup_with_automation` now accepts an optional
   `uninteresting_log_record_types` table, allowing users to customise
   which log record types are suppressed from TSA publishing.

## Fixes

 * [kumo.crypto.aws_sign_v4](../reference/kumo.crypto/aws_sign_v4.md) had
   several issues with its SigV4 implementation:

     * The `x-amz-content-sha256` header logic was inverted: it was being
       added to the signed header set for every service *except* S3, when
       AWS actually requires it specifically for S3 (and does not expect
       it for most other services).  S3 requests now correctly include
       `x-amz-content-sha256` in the signed headers, and other services
       no longer have it added implicitly.
     * Header value canonicalization now implements the AWS *Trimall*
       rule (strip leading/trailing space and tab, collapse internal runs
       of space and tab to a single space, preserving whitespace inside
       quoted strings) rather than only trimming the ends.
     * The `host` header is now required to be supplied by the caller;
       previously a misleading empty `host:` placeholder would be signed
       if it was omitted.  Header names supplied by the caller are
       matched case-insensitively, so `Host` and `host` are both
       accepted.

   The implementation is now verified against vectors from the official
   AWS SigV4 test suite.  If you are calling this function for a non-S3
   service (for example SNS, SQS, or Firehose) and were also sending
   `x-amz-content-sha256` on the wire, you should now either pass it
   explicitly in `headers` so it is included in the signed set, or stop
   sending it on the wire to match the signed request.
   Thanks to @AdityaAudi! #522

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

