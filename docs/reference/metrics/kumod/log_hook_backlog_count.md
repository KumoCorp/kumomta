# log_hook_backlog_count

```
Type: Counter
Labels: logger
```
how many times processing of a log event hit the back_pressure in a hook.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.
