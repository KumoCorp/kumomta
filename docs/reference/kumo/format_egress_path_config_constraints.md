# kumo.format_egress_path_config_constraints

```lua
local text = kumo.format_egress_path_config_constraints(constraints)
```

{{since('dev')}}

Given a constraints table previously obtained from
[kumo.compute_egress_path_config_constraints](compute_egress_path_config_constraints.md),
returns a human-readable multi-line string describing the
steady-state throughput ceilings. This is the same rendering used
by the `ceilings:` block of
[kcli inspect-ready-q](../kcli/inspect-ready-q.md), so output from
the two surfaces is identical.

Example output:

```
ceilings:
  concurrent dispatchers: 32
    source: connection_limit
  message rate:           10 × 10/s = 100/s
    source: max_deliveries_per_connection × max_connection_rate
    declared: max_message_rate = 1000/s ← effectively unreachable
  connection rate:        10/s
    source: max_connection_rate
```
