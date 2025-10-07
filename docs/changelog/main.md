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

## Other Changes and Enhancements

 * [msg:check_fix_conformance](../reference/message/check_fix_conformance.md#fixing-8-bit-content)
   now supports optionally detecting and fixing 8-bit charsets.
 * [smtp_server_data](../reference/events/smtp_server_data.md) event enables
   once-per-transaction processing of a message and recipient list modification
   for alias expansion and legal capture.

## Fixes

 * smtp server would incorrectly return a 451 instead of a 452 status when
   `max_recipients_per_message` or `max_messages_per_connection` limits
   were exceeded.
