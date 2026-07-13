# process_cpu_usage_normalized

```
Type: Gauge
```
The sum of the process CPU usage for each CPU in the system, divided by the number of CPUs.


100% in this metric indicates that all CPU cores are 100% busy.

This metric is scoped to the service process, reflecting the CPU used only
by the process and not the system as a whole.

