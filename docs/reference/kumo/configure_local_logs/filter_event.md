# filter_event

{{since('2023.11.28-b5252a41')}}

Optional string. If provided, specifies the name of an event that should
be triggered to decide whether logs for a given message should be included
in this instance of local file logging.

The event will be passed the message that is being considered for logging
purposes.  The goal of the event is to return `true` if logging should
proceed or `false` otherwise.

You may access the message metadata to make that decision.

```lua
kumo.on('init', function()
  -- We only want logs for messages accepted via SMTP to land here
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs-smtp',
    filter_event = 'should_log_to_smtp_logs',
  }

  -- We want all other logs to land here
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs-other',
    filter_event = 'should_log_to_other',
  }
end)

kumo.on('should_log_to_smtp_logs', function(msg)
  return msg:get_meta 'reception_protocol' == 'ESMTP'
end)

kumo.on('should_log_to_other', function(msg)
  return msg:get_meta 'reception_protocol' ~= 'ESMTP'
end)
```


