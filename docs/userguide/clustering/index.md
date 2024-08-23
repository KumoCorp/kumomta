# Clustering

KumoMTA is designed to be used in high-volume sending environments where cluster capabilities are essential to effective and manageable scaling.

To address these needs, KumoMTA has several features and integrations designed around cluster management.

## Implementation Approach

The KumoMTA team works from a philosophy of "don't re-invent the wheel". To the degree possible KumoMTA is designed to work with existing solutions rather than implement our own version of existing tools.

For example: KumoMTA does not provide a configuration distribution or versioning tool because there are numerous existing options including Git, Puppet, Chef, K8s, etc.

## Vertical vs Horizontal Scaling

While many existing users coming from commercial MTAs with per-node licensing tend toward vertical scaling, KumoMTA can be either vertically or horizontally scaled.

Vertical scaling with high-performance hardware can see over ten million messages per hour on a single node, where horizontal scaling environments tend to aim for between two to four million messages per hour per node. Those leveraging orchestration solutions such as Kubernetes will likely prefer a horizontal scaling architecture.

## Shared Vs. Node-Specific Configuration

While it is possible to configure KumoMTA nodes in a cluster with distinct configurations on a per-node basis, the traffic shaping approach used in KumoMTA is most effective when all nodes share the same IP configuration using proxies.

In this approach the nodes share a common configuration (implemented using the preferred method of the user) with the `Egress_Source` configured to use a proxy for sending. Alternatively Reverse NAT or port forwarding could also be used.

KumoMTA supports both HAPROXY and SOCKS5, see [the Proxy page](../operation/proxy.md) for information on using KumoMTA with outbound proxies.

KumoMTA includes a SOCKS5 proxy implementation, see the [KumoProxy page](../operation/kumo-proxy.md) for more information.

## Managing Shared Secrets

One of the most straightforward ways to manage secrets such as DKIM keys and authentication credentials is using a Vault.

For more information on using Vault for shared secrets see our [Storing Secrets in Hashicorp Vault](../policy/hashicorp_vault.md) page.
