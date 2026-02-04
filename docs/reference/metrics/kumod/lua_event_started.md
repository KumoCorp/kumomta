# lua_event_started

```
Type: Counter
Labels: event
```
Incremented each time we start to call a lua event callback. Use lua_event_latency_count to track completed events.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.
