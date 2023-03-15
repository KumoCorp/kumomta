# `kumo.on('http_message_generated', function(message))`

Called by the HTTP injection API endpoint after generating a message, but prior
to injecting it into the queue.

The event handler will be passed a [Message](../message/index.md) object.

The HTTP injector does not add a `Received` header, but it will pre-set the
following meta values in the message:

* `"http_auth"` - will hold either the authenticated username or the peer IP
  address that satisfied the authentication check for the endpoint.

This event is the best place to carry out a number of important policy
decisions:

* DKIM signing via [message:dkim_sign](../message/dkim_sign.md).
* Assigning the `"campaign"`, `"tenant"` and/or `"queue"` meta values via [msg:set_meta](../message/set_meta.md)

You may use [kumo.reject](../kumo/reject.md) to raise an error to prevent this
message from being queued for delivery.

```lua
kumo.on('http_message_generated', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    key = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```
