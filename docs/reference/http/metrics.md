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

There are currently more than 100 available metrics. 
You can see the current list by querying the endpoint with no arguments:

```bash
curl http://localhost:8000/metrics
```


```txt
# HELP connection_count number of active connections
# TYPE connection_count gauge
connection_count{service="esmtp_listener"} 0
# HELP disk_free_bytes number of available bytes in a monitored location
# TYPE disk_free_bytes gauge
disk_free_bytes{name="data spool"} 15658725376
disk_free_bytes{name="log dir /var/log/kumomta"} 15658725376
disk_free_bytes{name="meta spool"} 15658725376
# HELP disk_free_inodes number of available inodes in a monitored location
# TYPE disk_free_inodes gauge
disk_free_inodes{name="data spool"} 3056405
disk_free_inodes{name="log dir /var/log/kumomta"} 3056405
disk_free_inodes{name="meta spool"} 3056405
# HELP disk_free_inodes_percent percentage of available inodes in a monitored location
# TYPE disk_free_inodes_percent gauge
disk_free_inodes_percent{name="data spool"} 94
disk_free_inodes_percent{name="log dir /var/log/kumomta"} 94
disk_free_inodes_percent{name="meta spool"} 94
# HELP disk_free_percent percentage of available bytes in a monitored location
# TYPE disk_free_percent gauge
disk_free_percent{name="data spool"} 60
disk_free_percent{name="log dir /var/log/kumomta"} 60
disk_free_percent{name="meta spool"} 60
# HELP log_submit_latency latency of log event submission operations
# TYPE log_submit_latency histogram
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.005"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.01"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.025"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.05"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.1"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.25"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="0.5"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="1"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="2.5"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="5"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="10"} 0
log_submit_latency_bucket{logger="dir-/var/log/kumomta",le="+Inf"} 0
log_submit_latency_sum{logger="dir-/var/log/kumomta"} 0
log_submit_latency_count{logger="dir-/var/log/kumomta"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.005"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.01"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.025"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.05"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.1"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.25"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="0.5"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="1"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="2.5"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="5"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="10"} 0
log_submit_latency_bucket{logger="hook-http://127.0.0.1:8008.tsa.kumomta",le="+Inf"} 0
log_submit_latency_sum{logger="hook-http://127.0.0.1:8008.tsa.kumomta"} 0
log_submit_latency_count{logger="hook-http://127.0.0.1:8008.tsa.kumomta"} 0
# HELP lua_count the number of lua contexts currently alive
# TYPE lua_count gauge
lua_count 1
# HELP lua_event_latency how long a given lua event callback took
# TYPE lua_event_latency histogram
lua_event_latency_bucket{event="context-creation",le="0.005"} 0
lua_event_latency_bucket{event="context-creation",le="0.01"} 3
lua_event_latency_bucket{event="context-creation",le="0.025"} 3
lua_event_latency_bucket{event="context-creation",le="0.05"} 3
lua_event_latency_bucket{event="context-creation",le="0.1"} 3
lua_event_latency_bucket{event="context-creation",le="0.25"} 3
lua_event_latency_bucket{event="context-creation",le="0.5"} 3
lua_event_latency_bucket{event="context-creation",le="1"} 3
lua_event_latency_bucket{event="context-creation",le="2.5"} 3
lua_event_latency_bucket{event="context-creation",le="5"} 3
lua_event_latency_bucket{event="context-creation",le="10"} 3
lua_event_latency_bucket{event="context-creation",le="+Inf"} 3
lua_event_latency_sum{event="context-creation"} 0.017253747
lua_event_latency_count{event="context-creation"} 3
lua_event_latency_bucket{event="init",le="0.005"} 0
lua_event_latency_bucket{event="init",le="0.01"} 0
lua_event_latency_bucket{event="init",le="0.025"} 0
lua_event_latency_bucket{event="init",le="0.05"} 0
lua_event_latency_bucket{event="init",le="0.1"} 0
lua_event_latency_bucket{event="init",le="0.25"} 0
lua_event_latency_bucket{event="init",le="0.5"} 1
lua_event_latency_bucket{event="init",le="1"} 1
lua_event_latency_bucket{event="init",le="2.5"} 1
lua_event_latency_bucket{event="init",le="5"} 1
lua_event_latency_bucket{event="init",le="10"} 1
lua_event_latency_bucket{event="init",le="+Inf"} 1
lua_event_latency_sum{event="init"} 0.442427973
lua_event_latency_count{event="init"} 1
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.005"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.01"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.025"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.05"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.1"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.25"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="0.5"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="1"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="2.5"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="5"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="10"} 0
lua_event_latency_bucket{event="kumo.tsa.suspension.subscriber",le="+Inf"} 0
lua_event_latency_sum{event="kumo.tsa.suspension.subscriber"} 0
lua_event_latency_count{event="kumo.tsa.suspension.subscriber"} 0
# HELP lua_load_count how many times the policy lua script has been loaded into a new context
# TYPE lua_load_count counter
lua_load_count 3
# HELP lua_spare_count the number of lua contexts available for reuse in the pool
# TYPE lua_spare_count gauge
lua_spare_count 0
# HELP memory_limit soft memory limit measured in bytes
# TYPE memory_limit gauge
memory_limit 1538067456
# HELP memory_usage number of bytes of used memory
# TYPE memory_usage gauge
memory_usage 320516096
# HELP thread_pool_parked number of parked(idle) threads in a thread pool
# TYPE thread_pool_parked gauge
thread_pool_parked{pool="localset"} 0
thread_pool_parked{pool="logging"} 1
# HELP thread_pool_size number of threads in a thread pool
# TYPE thread_pool_size gauge
thread_pool_size{pool="localset"} 1
thread_pool_size{pool="logging"} 1
# HELP tokio_budget_forced_yield_count Returns the number of times that tasks have been forced to yield back to the scheduler after exhausting their task budgets.
# TYPE tokio_budget_forced_yield_count counter
tokio_budget_forced_yield_count 0
# HELP tokio_elapsed Total amount of time elapsed since observing runtime metrics.
# TYPE tokio_elapsed counter
tokio_elapsed 1057.312763632
# HELP tokio_injection_queue_depth The number of tasks currently scheduled in the runtime’s injection queue.
# TYPE tokio_injection_queue_depth gauge
tokio_injection_queue_depth 0
# HELP tokio_io_driver_ready_count Returns the number of ready events processed by the runtime’s I/O driver.
# TYPE tokio_io_driver_ready_count counter
tokio_io_driver_ready_count 0
# HELP tokio_num_remote_schedules The number of tasks scheduled from outside of the runtime.
# TYPE tokio_num_remote_schedules counter
tokio_num_remote_schedules 0
# HELP tokio_total_busy_duration The amount of time worker threads were busy.
# TYPE tokio_total_busy_duration counter
tokio_total_busy_duration 0
# HELP tokio_total_local_queue_depth The total number of tasks currently scheduled in workers’ local queues.
# TYPE tokio_total_local_queue_depth gauge
tokio_total_local_queue_depth 0
# HELP tokio_total_local_schedule_count The number of tasks scheduled from worker threads.
# TYPE tokio_total_local_schedule_count counter
tokio_total_local_schedule_count 0
# HELP tokio_total_noop_count The number of times worker threads unparked but performed no work before parking again.
# TYPE tokio_total_noop_count counter
tokio_total_noop_count 0
# HELP tokio_total_overflow_count The number of times worker threads saturated their local queues.
# TYPE tokio_total_overflow_count counter
tokio_total_overflow_count 0
# HELP tokio_total_park_count The number of times worker threads parked.
# TYPE tokio_total_park_count counter
tokio_total_park_count 0
# HELP tokio_total_polls_count The number of tasks that have been polled across all worker threads.
# TYPE tokio_total_polls_count counter
tokio_total_polls_count 0
# HELP tokio_total_steal_count The number of tasks worker threads stole from another worker thread.
# TYPE tokio_total_steal_count counter
tokio_total_steal_count 0
# HELP tokio_total_steal_operations The number of times worker threads stole tasks from another worker thread.
# TYPE tokio_total_steal_operations counter
tokio_total_steal_operations 0
# HELP tokio_workers_count The number of worker threads used by the runtime.
# TYPE tokio_workers_count gauge
tokio_workers_count 1
# HELP total_connections_denied total number of connections rejected due to load shedding or concurrency limits
# TYPE total_connections_denied counter
total_connections_denied{service="esmtp_listener"} 0

```


