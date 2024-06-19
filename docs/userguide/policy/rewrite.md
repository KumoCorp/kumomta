# Rewriting Remote Server Responses

{{since('2023.11.28-b5252a41')}}

One common use case for senders is the need to rewrite/transpose certain remote server responses before processing them as bounces. For example, a sender may receive a temporary failure message indicating that a mailbox is full, but wants to treat the response as a permanent failure because they don't expect the full mailbox to be resolved within the message's retry window and don't want the message increasing the size of the queue.

In order to rewrite remote server responses, use the [smtp_client_rewrite_delivery_status](../../reference/events/smtp_client_rewrite_delivery_status.md) event.

The following example is from the [smtp_client_rewrite_delivery_status](../../reference/events/smtp_client_rewrite_delivery_status.md) page in the [Reference Manual](../../reference/index.md):

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

Note that the message specified will have `(kumomta: status was rewritten from 400 -> 500)` appended to it when logged to make it clear that a rewrite has ocurred.
