# proxy_outbound_connections_total

```
Type: Counter
Labels: listener, destination
```
Total number of outbound connections made to destinations.


!!! note
    This metric is subject to *pruning*, which means that it may age out and reset to zero when the corresponding internal resources idle- or age-out of the system.  This is a memory management measure to prevent otherwise unbounded growth of memory over time.

!!! info
    This metric has labels which means that the system will track the metric for each combination of the possible labels that are active.  Certain labels, especially those that correlate with source or destination addresses or domains, can have high cardinality.  High cardinality metrics may require some care and attention when provisioning a downstream metrics server.

This counter tracks connections by destination IP address.
Note: This can create high cardinality if your proxy connects to many
unique destinations. The metric uses a pruning counter registry to
mitigate memory impact.

