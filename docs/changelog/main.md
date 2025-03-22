# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* SMTP Server: TLS parameters will now be cached for up to 5 minutes at
  a time, making it easier for a server to pick up an updated certificate
  file. In prior versions, the TLS parameters would be held for the lifetime
  of the process, requiring a restart to pick up a changed certificate/key
  pair.

## Fixes

* Specifying `validation_options` for the shaping helper without explicitly
  setting the new `http_timeout` could lead to a `missing field` error when
  running `kumod --validate`.
