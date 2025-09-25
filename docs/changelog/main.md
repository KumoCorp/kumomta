# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

 * [try_next_host_on_transport_error](../reference/kumo/make_egress_path/try_next_host_on_transport_error.md)
   option to more aggressively retry failures that are either transport errors
   (eg: timeout) or are not definitively associated with the message (eg:
   response to commands in between transactions).
 * You may now specify outbound SMTP port numbers when configuring
   [make_queue_config().protocol](../reference/kumo/make_queue_config/protocol.md)
   with an `mx_list`.
 * You may now specify outbound SMTP port numbers when assigning either the
   `routing_domain` or the domain portion of the scheduled queue name using the
   `queue` meta item. #352
 * [kumo.dns.lookup_ptr](../reference/kumo.dns/lookup_ptr.md) function for looking
   up PTR records. Thanks to @kayozaki! #390
 * [kumo.mpsc.define](../reference/kumo.mpsc/define.md) function for advanced
   non-durable, non-persistent, in-memory queue processing.
 * [kumo.fs](../reference/kumo.fs/index.md) module for efficiently working with
   the filesystem.  The functions
   [kumo.read_dir](../reference/kumo/read_dir.md),
   [kumo.glob](../reference/kumo/glob.md) and
   [kumo.uncached_glob](../reference/kumo/uncached_glob.md) have been
   deprecated in favor of functions with the same names in `kumo.fs`.  In
   addition, a new [kumo.fs.open](../reference/kumo.fs/open.md) function that
   can create async capable file handles is now provided.
 * SMTP Receptions made via TLS now: #100
    * Show in the trace headers as ESMTPS/ESMTPSA along with the TLS version
      and cipher as a comment. eg: `with ESMTPS (TLSv1_3:TLS13_AES_256_GCM_SHA384)`
    * Are recorded as `tls_cipher`, `tls_protocol_version` and
      `tls_peer_subject_name` in the meta values for the message and in the
      `Reception` log record.
 * New
   [tls_required_client_ca](../reference/kumo/start_esmtp_listener/tls_required_client_ca.md)
   parameter to aid in configuring mTLS
 * HTTP endpoints now support decompressing `deflate` and `gzip` compressed
   request bodies, which helps to reduce bandwidth usage particularly with the
   injection API. Thanks to @dschaaff! #394
 * You may now consume HashiCorp Vault secrets stored with keys named something
   other than `key` by using the new optional `vault_key` field in a
   [KeySource](../reference/keysource.md). Thanks to @pankajrathi95! #399
 * Powerful MIME parsing API exposed to lua. Use
   [msg:parse_mime](../reference/message/parse_mime.md) to parse incoming
   messages, or [kumo.mimepart](../reference/kumo.mimepart/index.md) to parse
   and/or (re)build messages independently from the incoming message flow. #117
 * [kumo.generate_rfc3464_message](../reference/kumo/generate_rfc3464_message.md)
   can be used to generate RFC 3464 non-delivery-reports.
 * New `event_time` and `created_time` fields in [Log
   Record](../reference/log_record.md) provide sub-second time stamp
   granularity. #405
 * [kumo.encode](../reference/kumo.encode/index.md) is now a bit more relaxed
   about excess (but otherwise harmless) padding in the various
   `baseXX` encoding schemes.
 * `received_via` and `hostname` are now set in the message metadata for
   messages injected via HTTP. #417
 * `SMTPUTF8` and `8BITMIME` are now advertised by the ESMTP listener. If the
   next SMTP hop doesn't advertise these extensions and the current message is
   8bit, then the message will be marked as permanently failed with a reason
   explaining that the content is incompatible with the next hop.  Previously,
   we'd try to send the 8bit data anyway, and the remote host would respond
   with its own error informing of the incompatibility. See
   [ignore_8bit_checks](../reference/kumo/make_egress_path/ignore_8bit_checks.md)
   for more discussion on this topic and the ability to disable this send-time
   checking.  #327

## Fixes

 * `msg:check_fix_conformance` could panic when attempting to fix messages with
   broken base64 parts
 * The kumo `proxy-server` now increases its soft `NOFILE` limit to match its
   hard limit on startup (just as we do in `kumod` and `tsa-daemon`), which
   helps to avoid issues with running out of file descriptors when no explicit
   tunings have been deployed for the proxy server.
