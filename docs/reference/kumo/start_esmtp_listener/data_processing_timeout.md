# data_processing_timeout

{{since('2025.03.19-1d3f1f67')}}

Sets an upper bound on the time that the SMTP server will allow
for processing the DATA portion of an SMTP transaction.  This
time period covers internal processing of the DATA once it
has been received.

The default for this is `"5 minutes"`.

!!! note
    The behavior of this option is probably not quite what you might expect, so
    read carefully!

The primary purpose of this option is to prevent KumoMTA from taking final
responsibility for relaying a message if it has taken longer than the defined
`data_processing_timeout`.   **It does *not* guarantee that there will be a
response to DATA within the specified timeout.**

The main benefit of this is to avoid uncertainty with the injecting client: if
processing DATA takes longer than the timeout configured in the client, it is
likely that the client will treat the timeout as a transient failure and re-try
delivery.  If the message was actually accepted by KumoMTA, but just took a
little bit longer, then the overall result would be a duplicate send of the
same logical message content.

If `data_processing_timeout` is set to a duration that is just a little less
than the client timeout then the duplicate delivery risk in that situation is
mitigated.

The way this works is that the majority of DATA-time processing will be run
within the specified deadline, returning a `451 4.4.5 data_processing_timeout
exceeded` if the time limit is exceeded.  Depending on the server configuration
and workload, there may be some portions of processing that cannot be cancelled
if they take too long (most spooling configurations have a code path where the
IO cannot be timed out, for example).

If, by the time KumoMTA is ready to insert the message into the outbound
processing flow, the time elapsed exceeds the `data_processing_timeout` then
KumoMTA will unwind the reception of the message from the spool and issue a
transient failure to the client.

In the event that a message is "unwound" you will observe both a `Reception`
and an internal `Bounce` record for the message explaining that the insertion
failed during reception.

```lua
kumo.start_esmtp_listener {
  -- The default is 5 minutes
  data_processing_timeout = '5 minutes',
}
```



