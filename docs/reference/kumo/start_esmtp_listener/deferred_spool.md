# deferred_spool

!!! danger
    Enabling this option may result in loss of accountability for messages.
    You should satisfy yourself that your system is able to recognize and
    deal with that scenario if/when it arises.

When set to `true`, incoming messages are retained in memory until after
their first transient delivery failure.

This can have a dramatic impact on throughput by removing local storage I/O as
a bottleneck, but introduces a risk of forgetting about those messages if the
machine loses power or if the **kumod** process exits unexpectedly.

```lua
kumo.start_esmtp_listener {
  -- ..
  deferred_spool = false,
}
```


