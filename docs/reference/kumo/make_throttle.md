# `kumo.make_throttle(NAME, SPEC)`

{{since('2024.06.10-84e84b89')}}

Constructs and returns a named throttle object.  A throttle allows constraining
the rate at which an operation can occur, according to a Generic Cell Rate
Algorithm.

When used together with
[kumo.configure.redis_throttles()](configure_redis_throttles.md), multiple
nodes can contribute to and respect a limit configured across a cluster.

The *name* parameter is an arbitrary name that can be used to define the
purpose and scope of a throttle.  For example, you might define the purpose as
`throttle-ready-queue` and the scope to be a particular tenant.  In that case
you might generate a name like `throttle-ready-queue-TENANT_NAME`.  Multiple
throttle objects with the same name will increment and check the same underlying
throttle; the *name* parameter defines the throttle.

The *spec* parameter defines the permitted rate of the throttle, and has the
form `quantity/period` where quantity is a number and period can be a measure
of time.

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

The returned throttle object has the following methods:

## throttle:sleep_if_throttled()

Checks to see if an operation can proceed, incrementing and sleeping the
current action until the operation can proceed.

Returns a boolean value to indicate whether the action was throttled; if it
returns `true` then it was throttled and a delay was applied.

This is useful for example when throttling the reception rate. In the example
below, the incoming SMTP session is paused during `MAIL FROM` until the message
is permitted by two sets of throttles, and then allowed to continue:

```lua
kumo.on('smtp_server_mail_from', function(sender)
  -- Limit reception rate to 50/minute per unique sender
  local throttle = kumo.make_throttle(
    string.format('reception-rate-for-%s', sender),
    '50/minute'
  )
  throttle:sleep_if_throttled()

  -- Additionally, limit reception rate to 100/minute, regardless of the sender
  local throttle = kumo.make_throttle('reception-rate', '100/minute')
  throttle:sleep_if_throttled()
end)
```

## throttle:delay_message_if_throttled(msg)

This method is intended to be used in the
[throttle_insert_ready_queue](../events/throttle_insert_ready_queue.md) event.

It will evaluate the throttle, and if a delay is required, update the due
time on the message to reflect that.

```lua
kumo.on('throttle_insert_ready_queue', function(msg)
  -- limit each tenant to 1000/hr
  local tenant = msg:get_meta 'tenant'
  local throttle = kumo.make_throttle(
    string.format('tenant-send-limit-%s', tenant),
    '1000/hr'
  )
  throttle:delay_message_if_throttled(msg)
end)
```

## throttle:throttle()

Checks to see if an operation can proceed, and increments the count if it is permitted.
The returned value indicates the outcome and returns a table with the following fields:

* `throttled` - a boolean that indicates whether the operation was throttled or
  allowed. If `true`, the operation was throttled and should not be permitted
  to proceed.
* `limit` - The total limit of this particular named throttle. Equivalent to the
  `X-RateLimit-Limit` HTTP header that might be returned in various web services
  that implement throttling.
* `remaining` - the remaining limit of this particular named throttle. Equivalent to the
  `X-RateLimit-Remaining` HTTP header that might be returned in various web services
  that implement throttling.
* `reset_after` - the remaining duration until the limit will reset to its maximum capacity.
  Equivalent to the `X-RateLimit-Reset` HTTP that might be returned in various web
  services that implement throttling.
* `retry_after` - the time until the operation should be retried, or `nil` if
  the action was allowed.

This can be used to implement alternative strategies for the throttle delay.
For example, if you want to issue a generic transient failure when the limit
is exceeded you might do something like the following:

```lua
kumo.on('smtp_server_mail_from', function(sender)
  -- Limit reception rate to 50/minute per unique sender
  local throttle = kumo.make_throttle(
    string.format('reception-rate-for-%s', sender),
    '50/minute'
  )
  local result = throttle:throttle()
  if result.throttled then
    kumo.reject(451, '4.4.5 try again later')
  end
end)
```
