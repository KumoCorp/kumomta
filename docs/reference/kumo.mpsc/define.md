# define

```lua
kumo.mpsc.define(NAME, OPTIONAL_LIMIT)
```

{{since('dev')}}

Defines a new Multi-Producer-Single-Consumer (MPSC) queue, returning a *queue*
object.  An MPSC queue allows multiple producers to submit values to the queue
without contention, while allowing only a single consumer to efficiently wait
for and pull values back out of the queue.

The `NAME` parameter is a string specifying the name of the queue.

The `OPTIONAL_LIMIT` parameter is an optional non-zero integer value that
specifies the optional buffer size associated with the queue.

If `OPTIONAL_LIMIT` is omitted (or is `nil`), then the queue will be created as
an *unbounded queue*, which will accept new items until memory is exhausted.

If a buffer limit is provided, then the queue will be created as a *bounded
queue* which can only hold up to the specified number of items.  Attempting to
add items when the queue is full will either cause the submitting code to
block, raise an error or return a status code to reflect that the submission
cannot proceed, depending on the method used to submit the new value.

It is safe to call `kumo.mpsc.define` multiple times with the same name and
varying values of `OPTIONAL_LIMIT`; the first call to `kumo.mpsc.define` will
actually define the queue and its parameters, while subsequent calls with that
name will return that original queue.

!!! note
    If you need to change the `OPTIONAL_LIMIT` parameter, the service must be
    restarted for that change to take effect.

!!! warning
    MPSC queues are neither durable nor persistent; anything buffered up in the
    queue will be lost when the service is restarted or terminated.

!!! caution
    Take care when using unbounded queues as they have no inherent defense
    against consuming all available memory on the system if the rate of sending
    exceeds the rate at which the items are being processed.

A queue can hold any memoizable value, which is most lua values (excluding
coroutines, functions) and a number of bindings to rust data types exposed by
kumo's runtime.  Attempting to send incompatible values will result in a
runtime error.  It is technically possible to send the `nil` value to a queue,
but the various receiving methods cannot disambiguate `nil` from the queue
being closed, so you should avoid doing that.

```lua
local function get_queue()
  return kumo.mpsc.define 'my-example-queue'
end

kumo.on('init', function()
  -- Spawn a task that will process items that were sent to the queue.
  -- It will try to pull items in batches of up to 128 at a time
  kumo.spawn_task {
    event_name = 'my.queue.task',
    args = {},
  }
end)

kumo.on('my.queue.task', function(args)
  -- Get a reference to the unbounded queue we created during `init`
  local q = get_queue()
  while true do
    local batch = q:recv_many(128)
    print(string.format('got a batch of %d items', #batch))
    for idx, item in ipairs(batch) do
      print(string.format('item idx %d: %s', idx, item))
    end
  end
end)

-- Calling this function will submit an item to the queue
local function submit_item(item)
  local q = get_queue()
  q:send(item)
end
```

## The Queue Object

The Queue Object has the following methods

### queue:close

```lua
queue:close()
```

Closes a queue, preventing future sends from succeeding.

### queue:is_closed

```lua
CLOSED = queue:is_closed()
```

Returns `true` if the queue has been closed, or `false` otherwise.

### queue:is_empty

```lua
EMPTY = queue:is_empty()
```

Returns `true` if the queue is empty, or `false` otherwise.

!!! caution
    This method can only be called by the task that is processing the queue. It
    cannot successfully be called concurrently with an outstanding `recv`,
    `try_recv` or `recv_many` call because only a single consumer is allowed to
    operate on an MPSC queue.  This method will raise an error if it is unable
    to acquire exclusive access to the consumer side of the queue.

### queue:len

```lua
LENGTH = queue:len()
```

Returns the number of items in the queue.

!!! caution
    This method can only be called by the task that is processing the queue. It
    cannot successfully be called concurrently with an outstanding `recv`,
    `try_recv` or `recv_many` call because only a single consumer is allowed to
    operate on an MPSC queue.  This method will raise an error if it is unable
    to acquire exclusive access to the consumer side of the queue.


### queue:send

```lua
queue:send(VALUE)
```

Sends `VALUE` into the queue. `VALUE` can be any memoizable value, as described above.

For bounded queues, `queue:send` will wait for there to be room in the queue
before returning.  No waiting occurs for unbounded queues, because there is no
limit on the capacity of the queue, so there is conceptually always room
available.

A runtime error will be generated if `VALUE` is not memoizable, if the queue
has been closed, or if some other kind of runtime resource error is
encountered.

```lua
queue:send 'hello'
queue:send { 1, 2, 3 }
queue:send(true)
```

### queue:send_timeout

```lua
queue:send_timeout(VALUE, TIMEOUT_SECONDS)
```

Sends `VALUE` into the queue, waiting up to `TIMEOUT_SECONDS` for there to be room for the item.

`VALUE` can be any memoizable value, as described above.

For bounded queues, `queue:send_timeout` will wait up to the specified number
of `TIMEOUT_SECONDS` (which can be fractional) for there to be room in the
queue before returning.  If `TIMEOUT_SECONDS` passes and no room is available,
a runtime error is generated.

For unbounded queues, `TIMEOUT_SECONDS` is ignored and this method behaves
identically to `queue:send`.

A runtime error will be generated if `VALUE` is not memoizable, if the queue
has been closed, or if some other kind of runtime resource error is
encountered.

```lua
-- Will raise an error if no room is available within 0.5 seconds
queue:send_timeout('hello', 0.5)
```

### queue:try_send

```lua
SUCCESS = queue:try_send(VALUE)
```

Sends `VALUE` into the queue, if there is room. `VALUE` can be any memoizable
value, as described above.

Returns `true` if the item was submitted to the queue, `false` otherwise.

For bounded queues this method will only succeed if there is room in the queue
at the moment `queue:try_send` is called.

A runtime error will be generated if `VALUE` is not memoizable.

This method can return false if the queue has been closed, or if some other
kind of runtime resource error is encountered.

```lua
local ok = queue:try_send(VALUE)
if not ok then
  -- queue is full
end
```

### queue:recv

```lua
ITEM = queue:recv()
```

Receives a value from the queue.  If the queue is empty, this method will sleep
indefinitely, until a value is submitted.

Returns `nil` if the queue has been closed.

It is recommended that you spawn a task to process values in a loop:

```lua
local function get_queue()
  return kumo.mpsc.define 'my-example-queue'
end

kumo.on('init', function()
  -- Spawn a task that will process items that were sent to the queue.
  -- It will try to pull items in batches of up to 128 at a time
  kumo.spawn_task {
    event_name = 'my.queue.task',
    args = {},
  }
end)

kumo.on('my.queue.task', function(args)
  -- Get a reference to the unbounded queue we created during `init`
  local q = get_queue()
  while true do
    local item = q:recv()
    if item then
      print('got', item)
    else
      break
    end
  end
end)
```

!!! caution
    This method can only be called by the task that is processing the queue. It
    cannot successfully be called concurrently with any other outstanding
    `try_recv` or `recv_many` call because only a single consumer is allowed to
    operate on an MPSC queue.  This method will raise an error if it is unable
    to acquire exclusive access to the consumer side of the queue.

### queue:try_recv

```lua
ITEM = queue:try_recv()
```

Attempts to receive a value from the queue.  If the queue is empty, or has been
closed, this method will immediately return `nil`.

!!! note
    It is NOT recommended to `queue:try_recv` in a loop, as that will result
    in a busy loop that will consume a lot of CPU.

```lua
local item = q:try_recv()
if item then
  print('got', item)
end
```

!!! caution
    This method can only be called by the task that is processing the queue. It
    cannot successfully be called concurrently with any other outstanding
    `recv` or `recv_many` call because only a single consumer is allowed to
    operate on an MPSC queue.  This method will raise an error if it is unable
    to acquire exclusive access to the consumer side of the queue.


### queue:recv_many

```lua
BATCH = queue:recv_many(LIMIT)
```

Attempts to receive an array of values from the queue.  If the queue is empty,
will wait indefinitely for an item to be submitted.  A maximum of `LIMIT`
values will be returned at once; if the queue holds more than `LIMIT` items,
the excess will remain in the queue.  If the queue holds less than `LIMIT`
items, but more than `0`, then those items will be immediately returned and no
waiting will occur.

Returns `nil` if the queue has been closed.

```lua
local function get_queue()
  return kumo.mpsc.define 'my-example-queue'
end

kumo.on('init', function()
  -- Spawn a task that will process items that were sent to the queue.
  -- It will try to pull items in batches of up to 128 at a time
  kumo.spawn_task {
    event_name = 'my.queue.task',
    args = {},
  }
end)

kumo.on('my.queue.task', function(args)
  -- Get a reference to the unbounded queue we created during `init`
  local q = get_queue()
  while true do
    local batch = q:recv_many(128)
    print(string.format('got a batch of %d items', #batch))
    for idx, item in ipairs(batch) do
      print(string.format('item idx %d: %s', idx, item))
    end
  end
end)

kumo.on('shutdown_logging', function()
  local q = get_queue()
  q:close()
end)
```

!!! caution
    This method can only be called by the task that is processing the queue. It
    cannot successfully be called concurrently with any other outstanding
    `recv` or `try_recv` call because only a single consumer is allowed to
    operate on an MPSC queue.  This method will raise an error if it is unable
    to acquire exclusive access to the consumer side of the queue.

