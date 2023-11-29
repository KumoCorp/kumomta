# `kumo.on('smtp_client_rewrite_delivery_status', function(response, domain, tenant, campaign, routing_domain))`

{{since('2023.11.28-b5252a41')}}

This event is triggered by the SMTP client if a message is rejected by
a destination SMTP server.

Rejected means one of:

* `MAIL FROM` or `RCPT TO` got a non-2xx response
* `DATA` got a non-354 response
* The final `CRLF.CRLF` got a non-250 response

In that circumstance, the SMTP response is formatted into a single line
(replacing CRLF with the literal `\r\n` sequence) and the queue name parameters
are extracted in order to call the `smtp_client_rewrite_delivery_status` event.

The purpose of the event is to enable you to make a policy decision to optionally
rewrite the status code.  For example, you may wish to treat a full mailbox as
a permanent failure if the nature of the message is some kind of bulk notification
that won't be missed.

The event can return the its own version of the SMTP status code which will be
used when considering what to do with the message.

For instance, if the event returns a 500 code when the original code was a 400
code, the message will be logged as a `Bounce` and then removed from the spool.

If the event returns `null`, or the same value as the original response, then
the message is processed as normal.

Otherwise, the response string will have `(kumomta: status was rewritten from
400 -> 500)` appended to it, so that it is clear from the logs that a rewrite
occurred.

This example shows how to build up a mapping table from a json file with contents
like this:

```json
{
  "4\\.2\\.1 <.+>: Recipient address rejected: this mailbox is inactive and has been disabled": 500,
  "4\\.2\\.2 The email account that you tried to reach is over quota\\.": 500
}
```

```lua
-- Compile a classifier from the json file; refresh it it every 5 minutes
local get_dsn_classifier = kumo.memoize(function()
  local data = kumo.json_load '/tmp/dsn_rewrite.json'
  return kumo.regex_set_map.new(data)
end, {
  name = 'dsn_rewrite',
  ttl = '5 minutes',
  capacity = 1,
})

-- This example ignores the queue name parameters, but you could get more
-- sophisticated and use those to define rules on a per-domain/tenant/campaign basis
-- if required
kumo.on(
  'smtp_client_rewrite_delivery_status',
  function(response, domain, tenant, campaign, routing_domain)
    local map = get_dsn_classifier()
    -- Match the classifier against the response.
    -- This will return the rewritten code if any, or null otherwise.
    -- We can simply return the result of the lookup directly.
    return map[response]
  end
)
```

See also:

 * [kumo.memoize](../kumo/memoize.md)
 * [kumo.regex_set_map.new](../kumo.regex_set_map/new.md)
