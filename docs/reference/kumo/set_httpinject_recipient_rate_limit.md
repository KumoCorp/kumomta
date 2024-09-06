# `kumo.set_httpinject_recipient_rate_limit(SPEC)`

{{since('dev')}}

Configures an optional throttle for the HTTP injection API. The *SPEC*
parameter may be `nil` to clear the rate limit, or be a throttle specification
as described in [kumo.make_throttle](make_throttle.md).

It is recommended that you configure this in the `pre_init` event, which triggers
prior to the `init` event where you start up your http listeners:

```lua
kumo.on('pre_init', function()
  kumo.set_httpinject_recipient_rate_limit 'local:10,000/s'
end)
```

The effect of setting the rate limit is that, for an incoming HTTP injection
request, the number of recipients in the request is assessed against the throttle,
and the request is put to sleep until the throttle will admit that number of
recipients.

It applies across any and all HTTP listeners that have been defined.

This can therefore be used to set the upper bound on the HTTP injection rate.

This limit does *not* apply to
[kumo.api.inject.inject_v1](../kumo.api.inject/inject_v1.md).

