# smtp_server_rewrite_response

```lua
kumo.on(
  'smtp_server_rewrite_response',
  function(status, response, command, conn_meta) end
)
```

{{since('2025.12.02-67ee9e96')}}

This event is triggered by the SMTP server just prior to sending a
response to the connected client.  It enables you to adjust the response
that is sent back to the client.

The parameters are:

* `status` - the SMTP status code (eg: 250, 400, etc.)
* `response` - the response message text that will be sent to the client. (eg: `"OK"`, `"4.4.5 data_processing_timeout exceeded"`)
* `command` - the SMTP command that is being responded to. This may be `nil`
  for unknown commands, or for unilateral responses.
* `conn_meta` - the [Connection Metadata](../connectionmeta.md) object for the connection

The return value is expected to be a two-tuple consisting of:

* The revised `status` value, or `nil` to retain the current status.
* The revised `response` value, or `nil` to retain the current response.

Not explicitly returning anything is equivalent to returning `nil, nil` and
preserving the current status and response.

The `response` string may be multi-line, separated by `\n`.  Multiline
responses in this context do not include the SMTP status code as a prefix;
KumoMTA will insert any SMTP continuation as appropriate so that you can focus
on just the textual content in the response string.

Any errors raised within the `smtp_server_rewrite_response` will be logged, but will
otherwise be treated as though `nil, nil` was returned and leave the status
unchanged.  In particular, this means that you cannot use `kumo.reject` to break
the connection at this stage.

!!! danger
    While you can change the status code that is returned to the client through
    this event, you should avoid changing the semantics of the transaction.
    For example, if you edit the DATA response from 250 to a failure code, that
    will not change the fact that the system has spooled and queued the message
    for delivery, and that particular status change will likely cause the
    client to retry delivery later, leading to duplicate delivery.

Changing the `status` to `421` will cause the connection to be closed on the
client.

## Example: amending the DATA response

This example prefixes `"super fantastic!"` to the successful SMTP DATA response case,
but leaves every other status as-is.

```lua
kumo.on(
  'smtp_server_rewrite_response',
  function(status, response, command, conn_meta)
    if command == 'DATA' and status == 250 then
      return 250, 'super fantastic!\n' .. response
    end

    -- leave the status and response unchanged
  end
)
```

Results in something like this being sent back to the client in the SMTP session:

```
250-super fantastic!
250 OK ids=edd8d788b4a111f098b2cc28aa0a5c5a
```
