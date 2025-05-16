# kumo.time.start_timer

```lua
kumo.time.start_timer(LABEL)
```

{{since('2025.01.23-7273d2bc')}}

Starts a timer with a specific label and returns a timer object.

The timer object can be used to update a latency histogram that is reported in
the prometheus metrics for the server to track how long it takes for a certain
operation to complete.

The most basic usage looks like this:

```lua
local timer = kumo.time.start_timer 'my-operation'

-- do something here
kumo.time.sleep(1.5)

-- And record the latency
timer:done()
```

After this runs, you will see the following metrics:

```console
$ curl -s 'http://127.0.0.1:8000/metrics' | grep user_lua
# HELP user_lua_latency how long something user-defined took to run in your lua policy
# TYPE user_lua_latency histogram
user_lua_latency_bucket{label="my-operation",le="0.005"} 1
user_lua_latency_bucket{label="my-operation",le="0.01"} 1
user_lua_latency_bucket{label="my-operation",le="0.025"} 1
user_lua_latency_bucket{label="my-operation",le="0.05"} 1
user_lua_latency_bucket{label="my-operation",le="0.1"} 1
user_lua_latency_bucket{label="my-operation",le="0.25"} 1
user_lua_latency_bucket{label="my-operation",le="0.5"} 1
user_lua_latency_bucket{label="my-operation",le="1"} 1
user_lua_latency_bucket{label="my-operation",le="2.5"} 1
user_lua_latency_bucket{label="my-operation",le="5"} 1
user_lua_latency_bucket{label="my-operation",le="10"} 1
user_lua_latency_bucket{label="my-operation",le="+Inf"} 1
user_lua_latency_sum{label="my-operation"} 1.5
user_lua_latency_count{label="my-operation"} 1
```

You can use the `<close>` feature of lua to automatically trigger the `:done()` call
when the timer object falls out of scope.  This is useful for example to track how
long it takes to run a function:

```
local function mything()
  -- This `timer` will automatically report the duration of the `mything`
  -- function when it returns, so you don't need to litter the function
  -- with timer:done() calls for each return case below
  local timer <close> = kumo.time.start_timer("mything")

  if something then
    return
  end

  if something_else then
    return
  end

end
```

The `timer:done()` method returns the number of seconds that have elapsed
since the timer was started.
