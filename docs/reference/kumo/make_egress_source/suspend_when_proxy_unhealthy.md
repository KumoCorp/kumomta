# suspend_when_proxy_unhealthy

{{since('dev')}}

Optional table.

Automatically suspend this egress source for the specified duration
when the configured proxy server appears unreachable. The following
kinds of failure count toward this rule:

- The proxy is unreachable, refuses the connection, or times out
  (`ConnectError` with `is_proxy = true`).
- The proxy server itself reports a bind failure for the requested
  source address (`ProxyBindError`).

See [ha_proxy_server](ha_proxy_server.md) and
[socks5_proxy_server](socks5_proxy_server.md) for the configuration
that selects the proxy.

While a source is suspended, pool selection skips it, which allows
other sources in the pool to be used instead of delaying the message.
If every source in a pool is suspended, messages assigned to that
pool are delayed until the earliest suspension expires.

The configuration is a table with the following fields:

 * `trigger` - when to fire the rule. Defaults to `'Immediate'` — a
   single matching failure trips the rule. Use
   `{ Threshold = "N/period" }` (the same syntax used by TSA shaping
   rules) to tolerate transient noise: the rule fires only after `N`
   matching failures within the rolling window.
 * `duration` - how long the source stays suspended once the rule
   fires. Required.

```lua
suspend_when_proxy_unhealthy = {
  trigger = { Threshold = '3/5m' }, -- optional; defaults to Immediate
  duration = '10m',
}
```

Suspension is process-local — there is no cluster coordination.

Observability:

- The gauge
  [egress_source_health_suspended](../../metrics/kumod/egress_source_health_suspended.md)
  reads `1` for `{source, reason="ProxyUnhealthy"}` while the source
  is suspended.
- The counter
  [egress_source_health_suspensions_total](../../metrics/kumod/egress_source_health_suspensions_total.md)
  increments on `{source, reason="ProxyUnhealthy"}` each time the
  source transitions into the suspended state.
- The counter
  [egress_source_connection_failures_total](../../metrics/kumod/egress_source_connection_failures_total.md)
  increments on `{source, kind="ProxyUnhealthy"}` for every classified
  failure regardless of whether this rule is configured.

A transition into the suspended state is also logged at `WARN`; the
auto-clear is logged at `INFO`.

See also [suspend_when_unplumbed](suspend_when_unplumbed.md).

## Examples

Tolerate a few transient proxy hiccups by requiring N failures within
a rolling window before suspending. This is the typical configuration
for proxy-backed sources, since proxies sometimes flap briefly without
warranting the source being removed from pool selection.

```lua
kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    socks5_proxy_server = 'proxy.example.com:1080',
    socks5_proxy_source_address = '10.0.0.1',
    suspend_when_proxy_unhealthy = {
      trigger = { Threshold = '3/5m' },
      duration = '10m',
    },
  }
end)
```

If you're using the [sources helper](../../../userguide/configuration/sendingips.md),
you can define the same source using the following syntax:

{% call toml_data() %}
[source."ip-1"]
socks5_proxy_server = "proxy.example.com:1080"
socks5_proxy_source_address = "10.0.0.1"
suspend_when_proxy_unhealthy = { trigger = { Threshold = "3/5m" }, duration = "10m" }
{% endcall %}

If your proxy infrastructure is intended to be rock-solid and a single
failure indicates a real outage worth acting on immediately, omit
`trigger` (defaults to `'Immediate'`) so the first matching failure
trips the rule and removes the source from pool selection.

```lua
kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    socks5_proxy_server = 'proxy.example.com:1080',
    socks5_proxy_source_address = '10.0.0.1',
    suspend_when_proxy_unhealthy = {
      -- trigger defaults to 'Immediate'
      duration = '10m',
    },
  }
end)
```

Or, in the sources-helper TOML form:

{% call toml_data() %}
[source."ip-1"]
socks5_proxy_server = "proxy.example.com:1080"
socks5_proxy_source_address = "10.0.0.1"
suspend_when_proxy_unhealthy = { duration = "10m" }
{% endcall %}
