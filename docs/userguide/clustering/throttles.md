# Implementing Shared Throttles

When KumoMTA is deployed in a clustered environment using shared IPs the nodes will need to be able to use shared counters in order to adhere to traffic shaping rules.

In a clustered deployment KumoMTA can use Redis to store and track counters related to traffic shaping rules, and multiple nodes can leverage the same Redis server(s) to share common throttles.

For more information see the [configure_redis_throttles](../../reference/kumo/configure_redis_throttles.md) page in the Reference Manual.