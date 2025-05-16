# kumo.reject

```lua
kumo.reject(CODE, MESSAGE)
```

Calling `kumo.reject` will raise a lua error that will cause the
current ESMTP event to respond with the SMTP error code and message
that you specify.

```lua
kumo.on('smtp_server_mail_from', function(sender)
  kumo.reject(420, 'rejecting all mail, just because')
  -- this line is not reached
end)
```
