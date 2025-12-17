# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
 * The `Handlebars` template dialect now runs with recursive lookup
   for improved compatibility with other handlebars implementations.
 * `msg:check_fix_conformance()` can now detect and attempt to fix issues where
   the charset is invalid for parts that use transfer-encoding, by applying
   any charset detection options, falling back to UTF-8.

## Fixes

 * An SPF record containing U+200B (zero width space) could cause
   SPF record parsing to panic and the service to crash
 * MIME part body extraction did not always consider the charset for text parts

