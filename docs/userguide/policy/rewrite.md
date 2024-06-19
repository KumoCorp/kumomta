# Rewriting Remote Server Responses

{{since('2023.11.28-b5252a41')}}

One common use case for senders is the need to rewrite/transpose certain remote server responses before processing them as bounces. For example, a sender may receive a temporary failure message indicating that a mailbox is full, but wants to treat the response as a permanent failure because they don't expect the full mailbox to be resolved within the message's retry window and don't want the message increasing the size of the queue.

In order to rewrite remote server responses, use the [smtp_client_rewrite_delivery_status](../../reference/events/smtp_client_rewrite_delivery_status.md) event.

The following is a very simple example of hardcoding all rejections to a single response, for an example of a configurable implementation see the [smtp_client_rewrite_delivery_status](../../reference/events/smtp_client_rewrite_delivery_status.md) page in the [Reference Manual](../../reference/index.md):

```lua
kumo.on(
  'smtp_client_rewrite_delivery_status',
  function(response, domain, tenant, campaign, routing_domain)
    return "4\\.2\\.2 The email account that you tried to reach is over quota\\.": 500
  end
)
```

Note that the message specified will have `(kumomta: status was rewritten from 400 -> 500)` appended to it when logged to make it clear that a rewrite has ocurred.
