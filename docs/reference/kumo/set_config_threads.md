# kumo.set_config_threads

```lua
kumo.set_config_threads(N)
```

{{since('dev')}}

Specifies the maximum number of queue configuration refreshes
that may be performed concurrently.

The default value is `8`. Values less than `1` are treated as `1`.

Increasing this value allows more queue configurations to be
refreshed concurrently, reducing the impact of slow configuration
callbacks.

Larger values may increase resource consumption, particularly
when configuration callbacks perform network or database operations.