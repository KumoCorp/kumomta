# max_connection_rate

Optional string.

Specifies the maximum permitted rate at which connections can be established
from this source to the corresponding destination site.

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

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    max_connection_rate = '100/min',
  }
end)
```

If the throttle is exceeded and the delay before a connection be established
is longer than the `idle_timeout`, then the messages in the ready queue
will be delayed until the throttle would permit them to be delievered again.


