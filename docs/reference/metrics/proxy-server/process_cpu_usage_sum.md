# process_cpu_usage_sum

```
Type: Gauge
```
The sum of the process CPU usage for each CPU in the system, can add up to more than 100%.


Each CPU has a value from 0-100% busy; a value of 100% in this metric
indicates that the load is equivalent to one fully utilized CPU.

A multi-CPU system can report more than 100% in this metric; a dual-CPU
system reporting 200% indicates that both CPUs are fully utilized.

See process_cpu_usage_normalized for a version of this metric that scales from
0% (totally idle) to 100% (totally saturated).

This metric is scoped to the service process, reflecting the CPU used only
by the process and not the system as a whole.

