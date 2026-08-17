# egress_source_health_suspended

```
Type: Gauge
Labels: source, reason
```
`1` while an egress source is currently auto-suspended, `0` otherwise. Pool selection skips a source whose gauge is `1`, rolling the remaining suspension duration into the per-pool `min_delay`.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

Labels:
* `source` is the operator-defined egress source name (the
  `name` field of `kumo.make_egress_source`).
* `reason` indicates which rule's firing produced the
  current suspension:
    * `Unplumbed` — a `suspend_when_unplumbed` rule fired
      because the source's local `source_address` was not
      plumbed on this host. Plumb the address (or correct
      the configured `source_address`) to resolve.
    * `ProxyUnhealthy` — a `suspend_when_proxy_unhealthy`
      rule fired because the configured proxy server was
      unreachable or rejected the requested source address.
      Investigate the proxy service.

