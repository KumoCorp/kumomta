# kumo.time.now

```lua
kumo.time.now()
```

{{since('dev')}}

Returns current time object.

```lua
local now = kumo.time.now()
print(now:format('%s'))
```

would print

```
1754539055
```

```lua
now:elapsed()
```

!!!note
    This function does not guarantees monotonic behavior. Please refer to link below.

Returns a number representing the amount of time elapsed in nanoseconds, since instantiation.

```lua
local now = kumo.time.now()
kumo.time.sleep(0.1)
local ok, elapsed = pcall(now.elapsed, now)
if ok then
  print(string.format('elapsed %d ns', elapsed))
end
```

Reference: https://doc.rust-lang.org/std/time/struct.Instant.html#monotonicity


```lua
now:format(FORMAT)
```

!!!note
    Make sure to test your strftime syntax before using the function in production.

Returns date time format from given FORMAT syntax.

```lua
local now = kumo.time.now()
kumo.time.sleep(0.1)
local ok, datetime = pcall(now.format, now, '%Y-%m-%d %H:%M:%S.%3f')
if ok then
  print(string.format('date %s', datetime))
end
```

would print

```
date 2025-08-07 04:04:29.960
```

Reference: https://docs.rs/chrono/latest/chrono/format/strftime/index.html