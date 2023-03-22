# Configuring Message Routing

If you need to "smarthost" or route messages through another server, you can do this to put it in a queue whose domain is imap.server.hostname and it will resolve the MX/A -> ip address as normal.

```console
msg:set_meta('queue', 'imap.server.hostname') 
```

This should me located in a 'smtp_server_message_received' function like this:

```console
kumo.on('smtp_server_message_received', function(msg)
    ...
    msg:set_meta('queue', 'my.smarthost.com')

end)

```
