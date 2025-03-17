# `kumo.http.connect_websocket(URL)`

{{since('2024.06.10-84e84b89')}}

Constructs a websocket client.

`URL` is the URL to which the websocket will connect.
It must be either `ws://` or `wss://` rather than `http://`
or `https://`.

If the connection is successful, the return value is a tuple
of the resulting websocket stream and the initial HTTP response
returned from the URL.

```lua
kumo.on('init', function()
  kumo.spawn_task {
    event_name = 'my.websocket.task',
  }
end)

kumo.on('my.websocket.task', function()
  local stream, response = kumo.http.connect_websocket 'wss://example.com/'

  -- Show the initial response; note that there may not be anything
  -- significant contained in the initial response
  print(response:status_code(), response:status_reason())
  for k, v in pairs(response:headers()) do
    print('Header', k, v)
  end
  print(response:text())

  -- Now process data from the connection.
  -- This is an infinite loop, so you MUST use `kumo.spawn_task`
  -- to this websocket processor, otherwise you will harm the
  -- normal operation of the server process.
  while true do
    local data = stream:recv()
    print(kumo.json.parse(data))
  end
end)
```

## WebSocketStream Methods

The returned stream object has the following methods:

### client:recv()

Waits for and then returns the next packet sent by the server.
The returned data is a string.

### client:recv_batch(duration)

{{since('dev')}}

Waits for up to `duration` (which can either be a number of seconds, or a
duration string like `10s`) and collects however many messages are sent by the
peer into a batch, which is returned as an array style table.

If the socket disconnects and no messages have been received, an error is raised.

Otherwise, the messages received so far will be returned.

If no messages are returned within the specified duration, the return value
will be an empty table.


