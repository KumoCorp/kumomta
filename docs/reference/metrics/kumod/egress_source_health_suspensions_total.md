# egress_source_health_suspensions_total

```
Type: Counter
Labels: source, reason
```
Increments each time a source transitions into the auto-suspended state due to one of its `suspend_when_*` rules firing. Re-triggering the same rule while the suspension is already active does not increment this counter; it only counts state transitions.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

Labels:
* `source` is the operator-defined egress source name (the
  `name` field of `kumo.make_egress_source`).
* `reason` indicates which rule fired:
    * `Unplumbed` — a `suspend_when_unplumbed` rule fired
      because the source's local `source_address` was not
      plumbed on this host. Plumb the address (or correct
      the configured `source_address`) to resolve.
    * `ProxyUnhealthy` — a `suspend_when_proxy_unhealthy`
      rule fired because the configured proxy server was
      unreachable or rejected the requested source address.
      Investigate the proxy service.

