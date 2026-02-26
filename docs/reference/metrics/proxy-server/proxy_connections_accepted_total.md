# proxy_connections_accepted_total

```
Type: Counter
Labels: listener
```
Total number of incoming connections accepted by the proxy.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This counter increments each time a new client connection is accepted
by a proxy listener, before any SOCKS5 handshake begins.

