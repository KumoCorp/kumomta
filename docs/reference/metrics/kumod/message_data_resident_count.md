# message_data_resident_count

```
Type: Gauge
```
Total number of Message objects with body data loaded.


Tracks how many messages have their `data` resident
in memory.  This may be because they have not yet saved
it, or because the message is being processed and the
data is either required to be in memory in order to
deliver the message, or because logging or other
post-injection policy is configured to operate on
the message.

