---
description: "Attach tenant, campaign, or arbitrary custom metadata to a message at injection and include it in your delivery logs."
---

# How Do I Attach Custom Metadata (Tenant / Campaign) to a Message?

The standard pattern for **SMTP** injection is to pass a value in a header, copy it into message metadata, and (optionally) strip the header so it is not transmitted to the recipient. If you inject over the **HTTP API** instead, you can attach metadata directly on the request — see [below](#attaching-metadata-via-the-http-injection-api). Either way, metadata travels with the message through retries and can be logged, but is never sent on the wire.

## Tenant and campaign

The queue helper does this for you via header options:

```toml
# queues.toml
tenant_header = "X-Tenant"
remove_tenant_header = true
campaign_header = "X-Campaign"
remove_campaign_header = true
```

Or set it explicitly in Lua:

```lua
kumo.on('smtp_server_message_received', function(msg)
  local tenant = msg:get_first_named_header_value 'X-Tenant'
  if tenant then
    msg:set_meta('tenant', tenant)
  end
end)
```

## Arbitrary custom metadata

Capture any header into metadata and log it. This is cheaper than logging the header directly:

```lua
kumo.on('smtp_server_message_received', function(msg)
  msg:import_x_headers { 'x-client-id' }
end)
```

```lua
kumo.configure_local_logs {
  meta = { 'tenant', 'campaign', 'x_client_id' },
}
```

!!! note
    To see tenant/campaign in **log-hook** records, include them in `log_parameters` at injection. Inside a log hook the message is a log-record message and does not carry the original message's meta.

## Attaching metadata via the HTTP injection API

If you inject over the [HTTP injection API](../reference/http/kumod/api_inject_v1_post.md)
rather than SMTP, skip the header round-trip: each recipient accepts a
`metadata` object that is attached directly to the generated message.

```json
{
  "envelope_sender": "noreply@example.com",
  "content": "Subject: hello\n\nHello there",
  "recipients": [
    {
      "email": "recipient@example.com",
      "metadata": {
        "campaign_id": "promo-2026-q2",
        "user_segment": "premium"
      }
    }
  ]
}
```

These key/value pairs land under the `extra` metadata key, so read them from Lua
via the `extra` table rather than as top-level meta:

```lua
kumo.on('http_message_generated', function(msg)
  local extra = msg:get_meta('extra')
  if extra then
    msg:set_meta('campaign', extra.campaign_id)
  end
end)
```

Note that the queue helper's `tenant_header` / `campaign_header` options act on
headers, not on this `extra` metadata. To drive tenant/campaign from an HTTP
injection, either set the corresponding headers in `content` (or the `headers`
field) so the queue helper picks them up, or copy the values out of `extra` into
`tenant` / `campaign` meta in `http_message_generated` as shown above.

## See also

* [Configuring Queue Management](../userguide/configuration/queuemanagement.md)
* [Configuring Logging](../userguide/configuration/logging.md)
* [HTTP Injection API (POST /api/inject/v1)](../reference/http/kumod/api_inject_v1_post.md)
