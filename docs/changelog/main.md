# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * [msg:check_fix_conformance](../reference/message/check_fix_conformance.md#fixing-8-bit-content)
   now supports optionally detecting and fixing 8-bit charsets.

## Fixes

 * smtp server would incorrectly return a 451 instead of a 452 status when
   `max_recipients_per_message` or `max_messages_per_connection` limits
   were exceeded.
