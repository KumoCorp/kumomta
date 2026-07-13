# proxy_tls_handshake_failures_total

```
Type: Counter
Labels: listener
```
Total number of TLS handshake failures.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This counter increments when TLS is enabled on a listener and
the TLS handshake with a client fails.

