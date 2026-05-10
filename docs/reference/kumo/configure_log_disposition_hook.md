---
tags:
 - logging
---

# kumo.configure_log_disposition_hook

```lua
kumo.configure_log_disposition_hook { PARAMS }
```

Configures a Lua callback that is invoked synchronously as disposition log
records are generated. Unlike [kumo.configure_log_hook](configure_log_hook.md),
this does not create queued log-record messages for later delivery; instead,
KumoMTA calls the `log_disposition_NAME` event directly with the original
[Message](../message/index.md) and the generated [log record](../log_record.md).

This is intended for policy work that must inspect the original message while
it is still available, such as generating non-delivery reports from bounce
records.

```lua
kumo.on('init', function()
  kumo.configure_log_disposition_hook {
    name = 'ndr_generator',
    per_record = {
      Any = {
        enable = false,
      },
      Bounce = {
        enable = true,
      },
    },
  }
end)

kumo.on('log_disposition_ndr_generator', function(msg, log_record)
  -- inspect msg and log_record here
end)
```

PARAMS is a Lua table that accepts the keys listed below.

## name

Required string naming the hook.

The name determines the event that is called for matching records. A hook named
`ndr_generator` calls the `log_disposition_ndr_generator` event.

## per_record

Optional table configuring which record types trigger this hook.

The keys of the `per_record` table must be log record types, or the special
`Any` key. For disposition hooks, `enable` is the only meaningful
`LogRecordParams` field; fields related to local file paths or formatting are
ignored because disposition hooks do not write log files or render log
templates.

See [configure_local_logs.per_record](configure_local_logs/per_record.md) for
the list of record type keys and the general `per_record` structure.

