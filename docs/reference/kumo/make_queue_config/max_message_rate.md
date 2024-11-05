# max_message_rate

{{since('2024.06.10-84e84b89')}}

Optional string.

Specifies the maximum permitted rate at which messages can move from this
scheduled queue and into the ready queue for the appropriate egress source.

The value is of the form `quantity/period`
where quantity is a number and period can be a measure of time.

Examples of throttles:

```
"10/s" -- 10 per second
"10/sec" -- 10 per second
"10/second" -- 10 per second

"50/m" -- 50 per minute
"50/min" -- 50 per minute
"50/minute" -- 50 per minute

"1,000/hr" -- 1000 per hour
"1_000/h" -- 1000 per hour
"1000/hour" -- 1000 per hour

"10_000/d" -- 10,000 per day
"10,000/day" -- 10,000 per day
```

Throttles are implemented using a Generic Cell Rate Algorithm.

If the throttle is exceeded the message will be re-inserted into the scheduled
queue with a delay based on the acceptance rate of the throttle.

This option is distinct from [the egress path
max_message_rate](../make_egress_path/max_message_rate.md) option in that this one
applies to a specific scheduled queue, whilst the egress path option applies to
the ready queue for a specific egress path, through which multiple scheduled
queues send out to the internet.

If you have configured `max_message_rate` both here and in an egress path,
the effective maximum message rate will be the lesser of the two values; both
constraints are applied independently from each other at different stages
of processing.
