---
description: "Ship KumoMTA logs to Splunk, Kafka, or another system using webhooks, Kafka, or AMQP instead of tailing the compressed local log files."
---

# How Do I Ship Logs to Splunk, Kafka, or Another System?

KumoMTA's local logs are written as zstd-compressed JSON segment files in `/var/log/kumomta`. There is no uncompressed-file or log-to-stdout option, and that is by design: the local logs are optimized for efficient on-disk storage, with the `tailer` utility available for ad hoc inspection.

To get log records into an external system (Splunk, ELK, Graylog, CloudWatch, Loki, a data warehouse), use one of KumoMTA's streaming integrations rather than trying to tail or copy the files:

* **Webhook / HTTP log hook** — POST JSON records to an HTTP collector.
* **Kafka** — publish records to a topic.
* **AMQP / RabbitMQ** — publish records to a queue.

These run alongside the local logs (they are additive; local logging continues if `configure_local_logs` is present), and each record is queued and retried like any other message.

## In-process vs. out-of-process shipping

The integrations above run **in-process**: each log event becomes a message in
KumoMTA's own queues, delivered and retried alongside outbound mail. That is
simple and needs no extra moving parts, but the events count against your relay
throughput (see [Log Hook Performance](../userguide/performance/loghooks.md)),
and a slow or unavailable collector applies backpressure to the same server
that is sending mail.

Alternatively, you can ship logs **out of process**. The
[`kumo.jsonl`](../reference/kumo.jsonl/index.md) module lets a separate,
long-lived script tail the on-disk zstd-JSONL segment files and forward them —
to a webhook or anywhere else — with its own checkpointing and retry, fully
decoupled from the main KumoMTA process. Because it reads the finished log
segments rather than enqueuing events, it never competes with mail delivery for
queue capacity, and it can be restarted or backfilled independently.
[`kumo.jsonl.new_tailer`](../reference/kumo.jsonl/new_tailer.md) provides
resumable, checkpointed reads; see its
[Batched Webhook Example](../reference/kumo.jsonl/new_tailer.md#batched-webhook-example)
for a worked webhook shipper.

Reach for the in-process hooks when you want the least configuration and your
collectors keep up; reach for the out-of-process tailer when you want log
shipping isolated from relay performance or need independent checkpoint/retry
control.

## Filtering what you ship

Filter inside the log-hook handler, not in the local logger. For example, to skip `Reception` records:

```lua
-- inside your log hook's send handler
if record.type == 'Reception' then
  return -- skip publishing this record
end
```

You can also use `should_enqueue_log_record` to decide which records are queued for delivery to the hook at all.

!!! note
    Each JSON line is a complete, self-contained record. Treat each line as the unit of ingestion; if you need a unique key per record, generate a time-based UUID.

## See also

* [Configuring Logging](../userguide/configuration/logging.md)
* [Viewing Logs](../userguide/operation/logs.md)
* [Publishing Log Events Via Webhooks](../userguide/operation/webhooks.md)
* [Kafka](../userguide/policy/kafka.md) · [AMQP](../userguide/policy/amqp.md)
* [kumo.jsonl.new_tailer](../reference/kumo.jsonl/new_tailer.md) — out-of-process log shipping
