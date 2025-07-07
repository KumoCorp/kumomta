# kumo.disconnect

{{since('dev')}}

```lua
kumo.disconnect(CODE, MESSAGE, REJECT_OR_DISCONNECT)
```

Calling `kumo.disconnect` will raise a lua error that will cause the
current ESMTP event to respond with the SMTP error code and message
that you specify, preferentially disconnecting the incoming SMTP session.

The parameters are:

* `CODE` - Required integer. The SMTP code to use alongside your `MESSAGE`.  Note that the SMTP
  protocol specification requires that a unilateral disconnection be sent with a
  `421` code.  You can adjust/account for your conformance via the optional
  `REJECT_OR_DISCONNECT` parameter detailed below.
* `MESSAGE` - Required string. The reason for the disconnection.  If you intend to use
  ENHANCED STATUS CODES, you should prefix the message with your enhanced
  status code.
* `REJECT_OR_DISCONNECT` - Optional string. Controls whether the connection will be disconnected
  or more simply rejected.  It accepts the following possible values:
   * `"ForceDisconnect"` - disconnect the session immediately after sending your
     `CODE` and `MESSAGE`, even if `CODE` is not `421`.  Using this value with
     a non-`421` code is technically non-conforming to the SMTP protocol
     specification.
   * `"FollowWith421"` - if the `CODE` is not `421`, emit the `CODE` and `MESSAGE`,
     then synthesize a `421 HOSTNAME disconnecting due to previous error` disconnection
     response to immediately follow it, and then disconnect.
   * `"If421"` - if the `CODE` is `421` disconnect the session. Otherwise, behave like
     [kumo.reject](reject.md) and leave the connection established.

### Disconnect with a 421

```lua
kumo.on('smtp_server_mail_from', function(sender)
  kumo.disconnect(421, 'mta.example.com go away!')
  -- this line is not reached
  -- The service responds to the peer with '421 mta.example.com go away!'
  -- and then drops the connection
end)
```

### Disconnect with a non-standard exit code, non-SMTP-conformant

This example is not strictly conforming to SMTP, which requires that a 421 be used to indicate a disconnection.
This may not matter in practice, but if it does matter for your use case, look at the example below this one.

```lua
kumo.on('smtp_server_mail_from', function(sender)
  kumo.disconnect(451, 'no thank you')
  -- this line is not reached
  -- The service responds to the peer with '451 no thank you'
  -- and then drops the connection
end)
```

### Disconnect with a non-standard exit code, while remaining SMTP conformant

```lua
kumo.on('smtp_server_mail_from', function(sender)
  kumo.disconnect(451, 'no thank you', 'FollowWith421')
  -- this line is not reached
  -- The service responds to the peer with '451 no thank you\r\n421 HOSTNAME disconnecting due to previous error'
  -- and then drops the connection
end)
```

