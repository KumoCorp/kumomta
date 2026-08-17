# egress_source_connection_failures_total

```
Type: Counter
Labels: source, kind
```
Counts connection failures classified as belonging to one of the source-health failure classes. Increments regardless of whether `suspend_when_*` is configured on the source, so an operator can observe the underlying signal before opting in to auto-suspension.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

{{since('dev')}}

Labels:
* `source` is the operator-defined egress source name (the
  `name` field of `kumo.make_egress_source`).
* `kind` is one of:
    * `Unplumbed` — the source's local `source_address`
      returned `EADDRNOTAVAIL` on `bind()`. The IP address
      is not currently plumbed on this host.
    * `ProxyUnhealthy` — the configured proxy server was
      unreachable, timed out, or reported a bind failure
      for the requested source address.

