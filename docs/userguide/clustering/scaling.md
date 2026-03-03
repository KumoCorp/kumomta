# Scaling Up and Down

One advantage of horizontal scaling in virtualized environments is that resources can be conserved by scaling the KumoMTA cluster up and down in response to sending patterns.

!!!Note
    Auto-scaling is a complicated subject that is not recommended except for those with extensive experience in autoscaling technologies. For most senders, the predictable daily sending patterns are adequate for scheduling scaling, with alerting when the cluster size is too small.

The most common install approach for scaling clusters involves Docker. For examples on deploying more advanced docker architectures, see [https://github.com/KumoCorp/kumomta/tree/main/examples](https://github.com/KumoCorp/kumomta/tree/main/examples).

While the exact implementation of a scaling cluster is up to the user and their tools of choice, KumoMTA offers several tools and APIs that can ease the process.

## Monitoring Node Availability

When a node is brought online there is some time required for the node to become ready for message injection.

To check on the availability of a node, you can check the `liveness` API on that node:

```console
$ curl -X GET "http://127.0.0.1:8000/api/check-liveness/v1"
```

A `200` response indicates that the node is available and ready to receive messages.

## Draining The Spool Before Shutdown

When a node needs to come offline the following steps can be used:

1. Remove the node from the load balancer to prevent future injections.
1. Call the [rebind](../../reference/http/kumod/api_admin_rebind_v1_post.md)
   endpoint of the KumoMTA API to redirect all messages back to the load
   balancer. In the following example the load balancer is an IP literal in
   square brackets, a hostname can be used without the square brackets:

    ```console
    $ kcli rebind --reason="Your reason here" --everything --set 'routing_domain=[192.168.1.100]'
    ```

1. Monitor the [metrics](../../reference/http/kumod/metrics_get.md) API
   endpoint to determine when the node's queues are empty, you can then shut
   down the node.
