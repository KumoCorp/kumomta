# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * New [message:import_headers](../reference/message/import_headers.md) method,
   a more flexible alternative to `message:import_x_headers`. Supports exact
   names and trailing-`*` wildcards, first/last/all match modes, optional
   removal of matched headers, and configurable name-transform styles
   (snake/kebab/camel/pascal case).

## Fixes

 * A message with multipart/mixed as the root with multipart/related as a child
   part was not structurally parsed correctly, producing incorrect parts when
   using [mimepart:get_simple_structure](../reference/mimepart/get_simple_structure.md).
   Thanks to @kayozaki! #506
 * typing.lua: couldn't distinguish `false` from unset for a boolean field with
   default of `true`, such as those used in `mail_auth.lua`. Thanks to
   @kayozaki! #505
 * Regression with `postmaster@domain` style addresses and null sender
   addresses when constructing messages via
   [kumo.make_message](../reference/kumo/make_message.md) and its equivalent
   internal API. Thanks in part to @kayozaki!  #511 #512
