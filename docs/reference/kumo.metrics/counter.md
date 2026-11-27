# kumo.metrics.counter

{{since('dev')}}

Defines a cumulative counter that can be scraped from `/metrics` and `/metrics.json`. Kumo adds `policy_` to the name. If `help` is omitted or empty, the description defaults to `name`. `labels` is an optional list of label names, such as `{ 'rule', 'result' }`. Prometheus reserves label names starting with `__`.

```lua
local checks = kumo.metrics.counter {
  name = 'rule_evaluations_total',
  help = 'Results of policy rules evaluated without enforcement',
  labels = { 'rule', 'result' },
}

-- Call from policy to see what requiring a List-Id header would reject.
local function observe_list_id(msg)
  local has_list_id = msg:get_first_named_header_value 'List-Id' ~= nil
  checks:inc {
    rule = 'require_list_id',
    result = has_list_id and 'pass' or 'would_reject',
  }
end
```

After observing one message without a `List-Id` header, `/metrics` includes:

```text
policy_rule_evaluations_total{result="would_reject",rule="require_list_id"} 1
```

`counter:inc(labels, amount)` adds a nonnegative integer, defaulting to one. Supply a string value for each declared label. For an unlabelled counter, call `counter:inc()` or `counter:inc({}, amount)`.

The first `:inc()` for each combination of label values creates a series that stays in memory until `kumod` exits. Choose labels with a small set of values, such as rule names and outcomes. Avoid values that change with every message, such as message IDs, because `kumod` will use more memory with each new value. To publish a series at zero before the first event, call `counter:inc(labels, 0)`.

## Policy reloads

All Lua contexts in a `kumod` process share the same counters. Reloading the policy preserves their counts. The first time you define a metric name, `kumod` remembers its help text and label names, even if you never increment it.

* **Editing the help text:** The counter keeps counting, but scrapes show the original help text until `kumod` restarts.
* **Changing the labels:** Reordering labels has no effect. If you add, remove, or rename a label under the same metric name, the policy still runs, but `:inc()` on the new definition records nothing. Counters using the original definition still work.

For each metric name, `kumod` logs one warning the first time only the help text changes and a separate warning the first time a label is added, removed, or renamed. To continue counting under the original name without losing counts, restore the original labels and reload. To use the new labels, choose a new metric name or restart `kumod` with the changed policy and fresh counts. Removing a definition from policy does not free its name or remove its published series before restart.
