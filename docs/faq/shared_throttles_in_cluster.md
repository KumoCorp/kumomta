# How Do Shared Throttles Work for Small Connection Limits in a Cluster?

One common challenge when sending to highly restrictive Mailbox Providers (MBPs) is that if the provider publishes a very low connection limit, you may have more nodes sharing an external IP than the provider allows for.

One common example is France's Orange, with a published connection limit of two connections at any time, as listed on their [Postmaster Page](https://postmaster.orange.fr/). The Orange guidelines are implemented in the default shaping.toml file as follows:

```toml
# https://messagerie.orange.fr/postmaster.html
["orange.fr"]
connection_limit = 2
max_deliveries_per_connection = 100
```

But what if you have multiple nodes sharing the same external IP address? 

If you have two nodes an option is to set each node to use a single connection. 

If you have more than two nodes a common workaround is to have a designated node for orange.fr that other nodes route all their traffic to, with the designated node configured to a two connection limit. This approach is of course quite fragile in that it depends on the availability of the designated node.

KumoMTA supports shared throttles when [installed as a cluster](../userguide/clustering/throttles.md) with a clustered Redis data store for throttle counters, but a common question is "what happens when there's more than two nodes?"

KumoMTA maintains a shared lease approach for connections from a given IP to a given destination in a clustered install. The lease is maintained and extended each time a message is sent.

The connection is closed and the lease released when the max deliveries on that connection is reached, or the connection idles out due to not having anything to do. This allows other nodes the opportunity to acquire the lease and send.

In this example `orange.fr` has a published limit of 100 messages per connection; when creating similar throttles a smaller number of messages per connection is desirable (even when the MBP allows for more messages per connection) because it allows for leases to be released more regularly.
