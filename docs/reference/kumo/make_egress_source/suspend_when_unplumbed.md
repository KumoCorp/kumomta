# suspend_when_unplumbed

{{since('dev')}}

Optional table.

Automatically suspend this egress source for the specified duration when
its [source_address](source_address.md) appears to be unplumbed on the
local host — that is, when `bind()` returns `EADDRNOTAVAIL` while
attempting to connect from this source.

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
suspend_when_unplumbed = {
  trigger = 'Immediate', -- optional; defaults to Immediate
  duration = '5m',
}
```

Suspension is process-local — there is no cluster coordination.

Observability:

- The gauge
  [egress_source_health_suspended](../../metrics/kumod/egress_source_health_suspended.md)
  reads `1` for `{source, reason="Unplumbed"}` while the source is
  suspended.
- The counter
  [egress_source_health_suspensions_total](../../metrics/kumod/egress_source_health_suspensions_total.md)
  increments on `{source, reason="Unplumbed"}` each time the source
  transitions into the suspended state.
- The counter
  [egress_source_connection_failures_total](../../metrics/kumod/egress_source_connection_failures_total.md)
  increments on `{source, kind="Unplumbed"}` for every classified
  failure regardless of whether this rule is configured, so you can
  observe the underlying signal before opting into auto-suspension.

A transition into the suspended state is also logged at `WARN`; the
auto-clear is logged at `INFO`.

See also [suspend_when_proxy_unhealthy](suspend_when_proxy_unhealthy.md).

## Examples

The common case for an unplumbed source: a single bind failure is
enough to know the interface isn't there, so use `Immediate` (the
default) and suspend for several minutes.

```lua
kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    source_address = '10.0.0.1',
    suspend_when_unplumbed = {
      -- trigger defaults to 'Immediate'
      duration = '5m',
    },
  }
end)
```

If you're using the [sources helper](../../../userguide/configuration/sendingips.md),
you can define the same source using the following syntax:

{% call toml_data() %}
[source."ip-1"]
source_address = "10.0.0.1"
suspend_when_unplumbed = { duration = "5m" }
{% endcall %}

To tolerate transient blips before suspending (e.g. on hosts where you
occasionally see a brief routing glitch), require N failures within a
rolling window with `Threshold`:

```lua
kumo.on('get_egress_source', function(source_name)
  return kumo.make_egress_source {
    name = source_name,
    source_address = '10.0.0.1',
    suspend_when_unplumbed = {
      trigger = { Threshold = '3/1m' },
      duration = '5m',
    },
  }
end)
```

Or, in the sources-helper TOML form:

{% call toml_data() %}
[source."ip-1"]
source_address = "10.0.0.1"
suspend_when_unplumbed = { trigger = { Threshold = "3/1m" }, duration = "5m" }
{% endcall %}
