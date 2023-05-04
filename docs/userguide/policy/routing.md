# Configuring Message Routing

If you need to "smarthost" or route messages through another server, you can do this to put it in a queue whose domain is imap.server.hostname and it will resolve the MX/A -> ip address as normal.

```lua
msg:set_meta('queue', 'imap.server.hostname')
```

This should me located in a 'smtp_server_message_received' function like this:

```lua
kumo.on('smtp_server_message_received', function(msg)
    ...
    msg:set_meta('queue', 'my.smarthost.com')
end)
```

You can also specify an IP address, but the queue still needs to be a valid domain per the SMTP spec, which requires [] around an ipv4 address, or that you use [IPv6:::1] for an IPv6 address:

```lua
kumo.on('smtp_server_message_received', function(msg)
    ...
    msg:set_meta('queue', '[20.83.209.56]')
end)
```
