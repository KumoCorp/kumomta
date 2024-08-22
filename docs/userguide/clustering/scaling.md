# Scaling Up and Down

One advantage of horizontal scaling in virtualized environments is that resources can be conserved by scaling the KumoMTA cluster up and down in response to sending patterns.

!!!Note
    Auto-scaling is a complicated subject that is not recommended except for those with extensive experience in autoscaling technologies. For most senders, the predictable daily sending patterns are adequate for scheduling scaling, with alerting when the cluster size is too small.

While the exact implementation of a scaling cluster is up to the user and their tools of choice, KumoMTA offers several tools and APIs that can ease the process.

## Monitoring Node Availability

When a node is brought online there is some time required for the node to become ready for message injection.

To check on the availability of a node, you can check the `liveness` API on that node:

```bash
curl -X GET "http://127.0.0.1:8000/api/check-liveness/v1"
```

A `200` response indicates that the node is available and ready to receive messages.

## Draining The Spool Before Shutdown

