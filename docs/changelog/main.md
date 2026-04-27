# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

## Fixes

 * A message with multipart/mixed as the root with multipart/related as a child
   part was not structurally parsed correctly, producing incorrect parts when
   using [mimepart:get_simple_structure](../reference/mimepart/get_simple_structure.md).
   Thanks to @kayozaki! #506
 * typing.lua: couldn't distinguish `false` from unset for a boolean field with
   default of `true`, such as those used in `mail_auth.lua`. Thanks to
   @kayozaki! #505
