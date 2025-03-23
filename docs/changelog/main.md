# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* SMTP Server: TLS parameters will now be cached for up to 5 minutes at
  a time, making it easier for a server to pick up an updated certificate
  file. In prior versions, the TLS parameters would be held for the lifetime
  of the process, requiring a restart to pick up a changed certificate/key
  pair.
* SMTP Server: new [via](../reference/kumo/start_esmtp_listener/via.md),
  [peer](../reference/kumo/start_esmtp_listener/peer.md), and
  [meta](../reference/kumo/start_esmtp_listener/meta.md) options for
  SMTP listeners enable metadata (and other existing listener options) to
  be conditionally set based on the source and local address of the
  incoming SMTP session.
* SMTP Server: new
  [smtp_server_connection_accepted](../reference/events/smtp_server_connection_accepted.md)
  event allows custom processing prior to returning the banner to the client.
* SMTP Server: new
  [smtp_server_get_dynamic_parameters](../reference/events/smtp_server_get_dynamic_parameters.md)
  event allows dynamically amending listener configuration to support IP-based
  virtual service.

## Fixes

* Specifying `validation_options` for the shaping helper without explicitly
  setting the new `http_timeout` could lead to a `missing field` error when
  running `kumod --validate`.
