# proxy_bytes_dest_to_client_total

```
Type: Counter
Labels: listener
```
Total bytes transferred from destination to client.


!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This counter tracks the total number of bytes flowing from destinations
back to proxy clients (downstream direction).

