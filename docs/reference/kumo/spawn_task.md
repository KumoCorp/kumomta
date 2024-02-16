# `kumo.spawn_task{PARAMS}`

{{since('dev')}}

!!! warning
    This function should be called only from inside your
    [init](../events/init.md) event handler.

This function will spawn a new thread that will trigger the specified event and
run it, allowing you to set up background tasks.

`PARAMS` is a lua table style object with the following fields:

* `event_name` - the name of the event which should be triggered in
  the new thread. You must register an event handler for this event
  using [kumo.on](on.md).
* `args` - an optional value that is passed to the event handler.
  You can use this to pass arguments to the event handler, which is
  useful in the case where you want to perform the same basic function
  in a task, but with varying parameters.

```lua
kumo.on('init', function()
  kumo.spawn_task {
    event_name = 'my.task',
    args = { 'hello', 'there' },
  }
end)

kumo.on('my.task', function(args)
  -- Prints: `I am the task.  ["hello","there"]`
  print('I am the task.', kumo.json_encode(args))
end)
```

!!! note
    If your task event handler raises an error, it will be logged and
    the task will stop. It is your responsibility to handle errors
    to ensure that your task remains running.  You can use the lua
    `pcall` function to trap errors and react accordingly.
