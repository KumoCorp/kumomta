# Unreleased Changes in The Mainline

## Breaking Changes

 * Enabling
   [`batch_handling="BatchByDomain"`](../reference/kumo/start_esmtp_listener/batch_handling.md)
   will cause [message:recipient](../reference/message/recipient.md) and the
   `recipient` field of [Log Record](../reference/log_record.md) to switch to
   an array holding the list of recipients.  These are *NOT* active by default,
   but if you wish to enable them you should audit your policy and consider
   switching to using
   [message:recipient_list](../reference/message/recipient_list.md) as well as
   review your log processors to ensure that they are able to handle the
   `recipient` field being either an array or a string, or otherwise adjusting
   your log templates accordingly.
 * HTTP injections no longer consider the `Forwarded` header as a source of
   information to populate the `received_from` metadata.  Instead, only the
   directly connecting IP information will be used.  See the [upstream
   issue](https://github.com/imbolc/axum-client-ip/issues/32) for more
   information.

## Other Changes and Enhancements

 * [msg:check_fix_conformance](../reference/message/check_fix_conformance.md#fixing-8-bit-content)
   now supports optionally detecting and fixing 8-bit charsets.
 * [smtp_server_data](../reference/events/smtp_server_data.md) event enables
   once-per-transaction processing of a message and recipient list modification
   for alias expansion and legal capture.
 * Admin bounces and scheduled queue suspensions can now optionally target the
   complete queue name instead of matching by domain/campaign/tenant.  This is
   useful in certain automation scenarios where you wish to target a specific
   queue precisely.  The kcli commands support a `--queue` option to select the
   queue name, while the API expose that via a `queue_names` field.
 * New [kcli xfer](../reference/kcli/xfer.md) and [kcli
   xfer-cancel](../reference/kcli/xfer-cancel.md) commands enable migration
   of queues to alternative kumomta nodes as part of operational tasks such
   draining a queue for decomissioning or scaling down infrastructure.  These
   commands are building blocks for you to deploy auto-scaling or similar
   functionality within your infrastructure orchestration. The new
   [xfer_message_received](../reference/events/xfer_message_received.md) can be
   used to fixup messages as they are arrive on the target node via xfer.
   `XferOut` and `XferIn` are two new [log record](../reference/log_record.md)
   types associated with message transfers. The kcli commands have
   corresponding HTTP API endpoints:
   [xfer](../reference/rapidoc.md/#post-/api/admin/xfer/v1) and
   [xfer-cancel](../reference/rapidoc.md/#post-/api/admin/xfer/cancel/v1) #311
 * New [kumo.file_type](../reference/kumo.file_type/index.md) module provides
   functions for reasoning about file types.
 * [kumo.amqp.build_client](../reference/kumo.amqp/build_client.md) is
   deprecated in favor of
   [kumo.amqp.basic_publish](../reference/kumo.amqp/basic_publish.md).
 * New [kumo.dns.ptr_host](../reference/kumo.dns/ptr_host.md),
   [kumo.dns.reverse_ip](../reference/kumo.dns/reverse_ip.md),
   [kumo.dns.define_resolver](../reference/kumo.dns/define_resolver.md) and
   [kumo.dns.rbl_lookup](../reference/kumo.dns/rbl_lookup.md) functions. #269
 * new `smtp_server_rejections` counter to track the number of `Rejection` log
   records produced by the smtp listener. The service key is the listener
   address and port, and there is a `total` key that represents the total across
   all listeners.

## Fixes

 * smtp server would incorrectly return a 451 instead of a 452 status when
   `max_recipients_per_message` or `max_messages_per_connection` limits
   were exceeded.
 * spf: a `NoRecordsFound` response from DNS during an `exists:` rule check
   could cause the result to incorrectly be reported a `temperror`
 * spf: `%{h}` macro expansion could incorrectly enclose the domain in double quotes
 * spf: relax macro parsing to allow spaces in, for example, explanation txt records
 * kumo.spf.check_host: `%{h}` will be assumed to have the value of the
   `domain` field when `sender` is not set, as `ehlo_domain` won't be set in
   the connection context until after `smtp_server_ehlo` returns successfully.
 * [kumo.start_esmtp_listener.line_length_hard_limit](../reference/kumo/start_esmtp_listener/line_length_hard_limit.md)
   could by off-by-two in certain cases when applied to DATA, and could
   sometimes allow up to 1024 bytes for a single SMTP command outside of DATA,
   even though the limit was set smaller.
 * Message builder API didn't quote every possible character that needed to be
   quoted in the display name of a mailbox. #428
