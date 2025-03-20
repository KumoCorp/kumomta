# `message:import_scheduling_header(HEADER_NAME, REMOVE)`

Reads the header specified and attempts to parse it as JSON-encoded version of
the [message:set_scheduling()](set_scheduling.md) parameters. If successful, it
will call [message:set_scheduling()](set_scheduling.md) with that value, and if
*REMOVE* is set to true, will remove the header from the message.

The return value of `message:import_scheduling_header` is:

* A lua table representation of the scheduling parameters, or nil if the specified header was not present. {{since('2025.03.19-1d3f1f67', inline=True)}}
* `nil` in prior versions of KumoMTA.

If the header is present, but has invalid contents the behavior will be:

* An error will be raised explaining the issue. {{since('2025.03.19-1d3f1f67', inline=True)}}
* No error indication will be given and no errors will be logged in prior versions of KumoMTA.

For example, given this message:

```
X-Schedule: {"dow":"Mon,Wed","tz":"America/Phoenix","start":"09:00:00","end":"17:00:00"}
Subject: hello

This message should only be delivered during working hours
in Phoenix on Mondays or Wednesdays.
```

this policy script will parse and remove that header, and apply the scheduling
constraints:

```lua
kumo.on('smtp_server_message_received', function(msg)
  msg:import_scheduling_header('X-Schedule', true)
end)
```
