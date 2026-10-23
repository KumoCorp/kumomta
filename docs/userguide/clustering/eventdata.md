---
description: Aggregate event data across a KumoMTA cluster using webhooks, AMQP, and Kafka instead of collecting and parsing local log files on every node.
---

# Aggregating Event Data

Clustered environments typically don't aggregate log files in order to collect event data, instead either processing the log files locally and then pushing the event data out, or using webhooks.

KumoMTA supports several methods commonly used for event data aggregating:

## Webhooks

KumoMTA can publish log information to HTTP endpoints in the form of webhooks, which can be received and consumed by log processing applications.

For more information see the [webhooks page](../operation/webhooks.md) of the User Guide.

## AMQP

KumoMTA also supports relaying log data via AMQP. 

For more information see the [AMQP page](../policy/amqp.md) of the User Guide.

## Kafka

KumoMTA supports publishing log events (as well as other messages) via Apache Kafka.

For more information see the [Kafka page](../policy/kafka.md) of the User Guide.