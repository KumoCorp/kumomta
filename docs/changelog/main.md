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

 * Fixed a remotely triggerable panic in MTA-STS policy handling. A destination
   domain could publish a policy with a `max_age` large enough that computing
   its expiry overflowed and aborted the process; because the triggering message
   stayed spooled and was retried, this crash looped on the outbound path. The
   `max_age` value is now clamped to the RFC 8461 maximum of 31557600 seconds.

 * Fixed a panic that aborted the process when `kumo.fs.glob` (or
   `kumo.glob`) was passed an absolute pattern containing a `**` recursive
   wildcard, such as `/opt/kumomta/etc/config/vmtas/**/*.toml`. Such patterns
   now match as expected and return absolute paths. Fixed by upgrading the
   `filenamegen` dependency to 0.2.8. #578

 * Fixed corruption of a multipart message whose body begins with a blank line
   or other preamble text before the first boundary. Adding a missing `Date`,
   `Message-ID`, or `MIME-Version` header via `msg:check_fix_conformance` wrote
   the `\r\n` in front of the first boundary back as `\n\r`, so the boundary no
   longer started a line and the altered bytes broke DKIM signatures. #607

 * Fixed `msg:check_fix_conformance` dropping the `Content-Disposition` header
   of a text part (such as a `text/calendar` invitation) when rebuilding the
   message. #604 #584

 * Fixed a remotely triggerable panic in DKIM verification. A message with two
   or more `DKIM-Signature` headers whose `b=` tags differed in length, one of
   them shorter than eight characters, panicked with an out-of-bounds read while
   computing the `header.b` authentication result. This was reachable on inbound
   mail through `msg:dkim_verify()`. The signature-count limit that caps
   verification work per message now also counts successfully parsed
   signatures, not only signatures that failed to parse.

 * Fixed an integer underflow when re-encoding a MIME parameter (such as a
   `Content-Type` parameter) whose name was long enough that the fold framing
   exceeded the target line width. This panicked in debug builds and produced
   a corrupt fold width in release builds. #608

 * Fixed a panic when folding a header line that contained a single run of
   text longer than the wrap limit with multi-byte (non-ASCII) characters,
   such as an internationalized address or a long UTF-8 word passed to
   `string.wrap()`. The over-long run was split byte by byte, which could
   cut a UTF-8 sequence in half and panic when the result was validated as
   UTF-8. It is now split on character boundaries.

 * Rebuilding a message header now recognizes `Authentication-Results` as a
   structured header, re-encoding it in canonical form rather than leaving it
   untouched as free text.

 * Constructed messages, including those built by the HTTP injection API, now
   emit the `MIME-Version` header with its uppercase spelling rather than
   `Mime-Version`. Both are valid per RFC 2045, but some spam filters such as
   rspamd score the mixed-case form. #564

 * Hardened display-name encoding when building messages: a CR or LF embedded
   in an address header display name (such as `From`, `To`, etc.),
   is now rewritten to a space to prevent it from splitting the header and
   injected a spurious header.  This is a robustness fix rather than
   a privilege escalation, as both the injection API and Lua policy already compose
   arbitrary message content and headers anyway.  This is only applicable
   to code that directly or indirectly calls into the builder API.

 * The HTTP injection API now encodes address headers correctly when the
   display name is non-ASCII. Given `Cc: 山田 <user@example.com>`, it now emits
   the name as an encoded-word and leaves the address untouched
   (`=?UTF-8?q?...?= <user@example.com>`); previously it encoded the whole
   value, address included, producing an invalid header. #598
