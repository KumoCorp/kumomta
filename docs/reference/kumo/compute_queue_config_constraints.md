# kumo.compute_queue_config_constraints

```lua
local constraints = kumo.compute_queue_config_constraints(queue_config)
```

{{since('dev')}}

Given a scheduled-queue configuration table (as returned by
[kumo.invoke_get_queue_config](invoke_get_queue_config.md) or
constructed via [kumo.make_queue_config](make_queue_config.md)),
returns the throughput ceilings implied by the queue config.

Today this surfaces `max_message_rate`, which gates promotion of
messages from the scheduled queue into its ready queue. The result
has the same shape as the value returned by
[kumo.compute_egress_path_config_constraints](compute_egress_path_config_constraints.md):
ceilings carrying a `value`, `source` and `display`, with `source`
tagged as `other` with name `"scheduled queue max_message_rate"`.

Pass the result back into
[kumo.compute_egress_path_config_constraints](compute_egress_path_config_constraints.md)
as its `additional` argument to fold the scheduled-queue rate into
the path-derived ceilings.
