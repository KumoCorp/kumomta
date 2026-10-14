# kumo.set_httpinject_concurrency_limit

```lua
kumo.set_httpinject_concurrency_limit(N)
```

{{since('dev')}}

Bounds the number of HTTP injection requests that may be processed
concurrently. *N* is the maximum number of in-flight requests and must be at
least 1. The default limit is `32768`.  Earlier versions of KumoMTA did
not impose a concurrency constraint, and would be limited only by system
resources (file descriptors, ports, RAM).

It is recommended that you set the concurrency limit in the `pre_init` event,
which triggers prior to the `init` event where you start up your http
listeners:

```lua
kumo.on('pre_init', function()
  kumo.set_httpinject_concurrency_limit(128)
end)
```

When a request arrives and the limit is already reached, it is rejected
immediately with a `503` response and the
[httpinject_requests_rejected_total](../metrics/kumod/httpinject_requests_rejected_total.md)
counter is incremented. The
[httpinject_requests_in_flight](../metrics/kumod/httpinject_requests_in_flight.md)
gauge reports how many requests currently hold a slot.

The concurrency limit is checked before the injection request is processed. The
request is then internally constrained by the
[recipient rate limit](set_httpinject_recipient_rate_limit.md).

It applies across all HTTP listeners that have been defined.

This limit covers only requests arriving over HTTP;
[kumo.api.inject.inject_v1](../kumo.api.inject/inject_v1.md) calls into the
same injection logic directly from Lua and is not constrained by it.

A request with `deferred_generation: true` releases its slot as soon as its body
is queued for generation. The later per-recipient generation is not constrained
by this limit. See [deferred_generation](../http/kumod/api_inject_v1_post.md)
for how to constrain that work.
