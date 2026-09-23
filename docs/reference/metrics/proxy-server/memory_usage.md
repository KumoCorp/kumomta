# memory_usage

```
Type: Gauge
```
number of bytes of used memory that drives memory limit decisions.  When sourced from a cgroup this is the working set  (memory.current - inactive_file, floored by anonymous memory);  otherwise it is the Resident Set Size from /proc/self/statm.

