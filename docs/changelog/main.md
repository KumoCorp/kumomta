# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * The HTTP injection API now supports per-recipient metadata via a new
   `metadata` field on each recipient object. Key-value pairs supplied
   there are stored in the message's `rcpt_meta` metadata field, making
   them accessible in Lua hooks using `msg:get_meta('rcpt_meta')`

## Fixes

 * A message with multipart/mixed as the root with multipart/related as a child
   part was not structurally parsed correctly, producing incorrect parts when
   using [mimepart:get_simple_structure](../reference/mimepart/get_simple_structure.md).
   Thanks to @kayozaki! #506
 * typing.lua: couldn't distinguish `false` from unset for a boolean field with
   default of `true`, such as those used in `mail_auth.lua`. Thanks to
   @kayozaki! #505
