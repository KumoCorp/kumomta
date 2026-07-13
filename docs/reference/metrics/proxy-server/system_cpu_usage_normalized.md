# system_cpu_usage_normalized

```
Type: Gauge
```
The sum of the system-wide CPU usage for each CPU in the system, divided by the number of CPUs.


100% in this metric indicates that all CPU cores are 100% busy.

This metric is scoped to the system, reflecting the total load on the
system, not just from the kumo related process(es).

