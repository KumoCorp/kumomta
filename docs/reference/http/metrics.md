# `GET /metrics`

Exports various counters, gauges and other metrics using the [Prometheus Text
Exposition
Format](https://prometheus.io/docs/instrumenting/exposition_formats/).

Access to this endpoint requires *Trusted IP* authentication. HTTP
authentication is not permitted.

See also [metrics.json](metrics.json.md).

{{since('2024.06.10-84e84b89', indent=True)}}
    You may specify an optional `prefix` GET parameter to have
    the reported metric names be prefixed with a string. For example,
    you might use `http://localhost:8000/metrics?prefix=kumomta_`.
    This can be helpful when it comes to matching or discovering
    kumomta specific metrics, especially in a busy prometheus
    instance.

## Example data

Here's an example of the shape of the data. The precise set of counters
will vary as we continue to enhance KumoMTA.

```txt
# HELP connection_count number of active connections
# TYPE connection_count gauge
connection_count{service="esmtp_listener"} 0
connection_count{service="smtp_client"} 0
connection_count{service="smtp_client:source2->"} 0
# HELP scheduled_count number of messages in the scheduled queue
# TYPE scheduled_count gauge
scheduled_count{queue="example.com"} 0
# HELP lua_count the number of lua contexts currently alive
# TYPE lua_count gauge
lua_count 1
# HELP lua_load_count how many times the policy lua script has been loaded into a new context
# TYPE lua_load_count counter
lua_load_count 1
# HELP lua_spare_count the number of lua contexts available for reuse in the pool
# TYPE lua_spare_count gauge
lua_spare_count 1
# HELP memory_limit soft memory limit measured in bytes
# TYPE memory_limit gauge
memory_limit 101234377728
# HELP memory_usage number of bytes of used memory
# TYPE memory_usage gauge
memory_usage 185647104
# HELP message_count total number of Message objects
# TYPE message_count gauge
message_count 1
# HELP message_data_resident_count total number of Message objects with body data loaded
# TYPE message_data_resident_count gauge
message_data_resident_count 1
# HELP message_meta_resident_count total number of Message objects with metadata loaded
# TYPE message_meta_resident_count gauge
message_meta_resident_count 1
# HELP ready_count number of messages in the ready queue
# TYPE ready_count gauge
ready_count{service="smtp_client:source1->loopback.dummy-mx.example.com"} 0
ready_count{service="smtp_client:source2->loopback.dummy-mx.example.com"} 0
# HELP total_connection_count total number of active connections ever made
# TYPE total_connection_count counter
total_connection_count{service="smtp_client"} 0
total_connection_count{service="smtp_client:source2->"} 0
# HELP total_messages_delivered total number of messages ever delivered
# TYPE total_messages_delivered counter
total_messages_delivered{service="smtp_client"} 0
total_messages_delivered{service="smtp_client:source2->"} 0
# HELP total_messages_fail total number of message delivery attempts that permanently failed
# TYPE total_messages_fail counter
total_messages_fail{service="smtp_client"} 0
total_messages_fail{service="smtp_client:source2->"} 0
# HELP total_messages_transfail total number of message delivery attempts that transiently failed
# TYPE total_messages_transfail counter
total_messages_transfail{service="smtp_client"} 0
total_messages_transfail{service="smtp_client:source2->"} 0
```
