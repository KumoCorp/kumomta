# message_meta_resident_count

```
Type: Gauge
```
Total number of Message objects with metadata loaded.


Tracks how many messages have their `meta` data resident
in memory.  This may be because they have not yet saved
it, or because the message is being processed and the
metadata is required for that processing.

