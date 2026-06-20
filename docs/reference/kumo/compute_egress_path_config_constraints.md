# kumo.compute_egress_path_config_constraints

```lua
local constraints = kumo.compute_egress_path_config_constraints(path_config)
```

{{since('dev')}}

Given an egress path configuration table (as returned by
[kumo.invoke_get_egress_path_config](invoke_get_egress_path_config.md)
or constructed via
[kumo.make_egress_path](make_egress_path/index.md)), returns the
steady-state throughput ceilings implied by that configuration.

The returned value has the following shape:

```lua
local example = {
  max_concurrent_dispatchers = {
    value = 32,
    source = { kind = 'primary' },
    display = '32',
  },
  max_message_rate = {
    value = 100,
    source = { kind = 'reconnect_cycling' },
    display = '10 × 10/s = 100/s',
  },
  max_message_rate_declared = '1000/s',
  max_connection_rate = {
    value = 10,
    source = { kind = 'primary' },
    display = '10/s',
  },
  max_source_selection_rate = nil,
}
```

Each axis carries an `EffectiveCeiling` with:

  * `value`: canonical numeric value (events per second for rate
    axes, count for concurrency).
  * `source`: tagged enum identifying which configuration term
    produced the ceiling. `kind` is one of `"primary"`,
    `"additional"` (with a `name` field naming the entry in the
    corresponding `additional_*` map), or `"reconnect_cycling"`.
  * `display`: human-readable string preserving the operator's
    original configuration units.

`max_message_rate_declared` is set only when `max_message_rate` is
explicitly configured but a different term (typically the
`reconnect_cycling` ceiling of
`max_deliveries_per_connection × max_connection_rate`) wins the
minimum. It records the declared rate so renderers can surface a
"declared but unreachable" annotation.

See also
[kumo.format_egress_path_config_constraints](format_egress_path_config_constraints.md)
for a human-readable rendering of the constraints, and
[kcli inspect-ready-q](../kcli/inspect-ready-q.md) which displays
the same diagnostic for a live ready queue.
