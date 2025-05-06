# log_oob

Affects how incoming RFC 3464 formatted out-of-band bounce report messages are
handled.

Can be one of the following values:

 * `"Ignore"` - do not parse or care whether the incoming message might
   be an OOB report. {{since('dev', inline=True)}}
 * `"LogThenRelay"` - if the incoming message is an OOB report, then
   log the `OOB` record and continue to allow the message to be
   enqueued for relay.  You will also see a `Reception` record for the relayed
   message, as well as records for its attempts to relay after reception.
   {{since('dev', inline=True)}}
 * `"LogThenDrop"` - if the incoming message is an OOB report, then log
   the `OOB` record, but silently drop the message without relaying it.
   There will be no additional log records for the message.
   {{since('dev', inline=True)}}
 * `false` - equivalent to `"Ignore"`.  This is for backwards compatibility
   with earlier versions of KumoMTA and we recommend using `"Ignore"` explicitly
   in your configuration moving forwards.
 * `true` - equivalent to `"LogThenRelay"`.  This is for backwards compatibility
   with earlier versions of KumoMTA and we recommend using `"LogThenRelay"`
   explicitly in your configuration moving forwards if you want that behavior,
   although you will likely prefer to use `"LogThenDrop"` in most cases.

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if domain == 'bounce.example.com' then
    return kumo.make_listener_domain {
      log_oob = 'LogThenDrop',
    }
  end
end)
```

