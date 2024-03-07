# `message:set_due(due)`

{{since('dev')}}

This method overrides the next delivery time for the message.  The *due* parameter
may be:

* `nil` - to indicate that delivery should be attempted as soon as possible.
* an ISO 8601 date and timestamp to specify the time of the next delivery attempt.
  `msg:set_due("2024-03-08T17:51:42.481711Z")`

Setting the due time is only valid in certain limited circumstances:

* Immediately at reception in either the
  [smtp_server_message_received](../events/smtp_server_message_received.md) or
  [http_message_generated](../events/http_message_generated.md) events, although
  it is much simpler to use the [message:set_scheduling](set_scheduling.md) method
  in almost all cases.
* During spooling in the
  [spool_message_enumerated](../events/spool_message_enumerated.md) event, where
  it is anticipated that re-binding a message for immediate delivery would be
  the most likely use-case.

!!! warning
    Using this method in any other way can result in non-deterministic,
    undefined and unsupported behavior.

