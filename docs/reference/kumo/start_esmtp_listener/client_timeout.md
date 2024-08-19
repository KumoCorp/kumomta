# client_timeout

Controls the timeout used when reading data from the client.
If no data arrives within the specified timeout, the server
will close the connection to the client.

```lua
kumo.start_esmtp_listener {
  -- The default is 1 minute
  client_timeout = '1 minute',
}
```


