# set_scheduling

```lua
message:set_scheduling { SCHED }
```

```lua
message:set_scheduling(nil)
```

Allows setting schedule constraints on the message.

When called with `nil` as a parameter, will clear any scheduling constraints
that are set on the message.

Otherwise, `SCHED` is a lua object that accepts a number of fields as listed below.
There are two separate groups of scheduling constraint:

* Deferred initial delivery, using the `first_attempt` field
* Constrained time/day of week delivery using the `dow`, `tz`, `start` and `end` fields.
* Custom expiration time, using the `expires` field. {{since('2025.03.19-1d3f1f67', inline=True)}}.

When using constrained time of delivery, all four of the associated fields must be
specified.  If not using constrained time of delivery, all four of the associated
fields must be omitted.

Constrained delivery modifies the normal exponential backoff retry schedule in
a simple way: the target time is computed as normal, and if that time does not
fall within the constrained delivery window, the scheduled time will be moved
to the next following date/time at which delivery will be acceptable. In
practice, that will be the `start` time on the follow appropriate `dow`.

The return value of `message:set_scheduling` is:

* A lua table representation of the scheduling parameters. {{since('2025.03.19-1d3f1f67', inline=True)}}
* `nil` in prior versions of KumoMTA.

Just setting the `first_attempt`:

```lua
msg:set_scheduling { first_attempt = '2023-03-01T17:00:00:00-08:00' }
```

setting constraints to deliver only on Mondays and Wednesdays during business
hours, Phoenix time.  Note that `end` has to be quoted to be used as a table
key in lua, because it is a language keyword:

```lua
msg:set_scheduling {
  dow = 'Mon,Wed',
  tz = 'America/Phoenix',
  start = '09:00:00',
  ['end'] = '17:00:00',
}
```

and both together:

```lua
msg:set_scheduling {
  first_attempt = '2023-03-01T17:00:00:00-08:00',
  dow = 'Mon,Wed',
  tz = 'America/Phoenix',
  start = '09:00:00',
  ['end'] = '17:00:00',
}
```

## first_attempt

Optional String.

If present, must be an [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339)
date/time string which specifies the earliest time at which the message will be
scheduled for delivery.

## expires

{{since('2025.03.19-1d3f1f67')}}

Optional String.

If present, must be an [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339)
date/time string which specifies the time at which the message will be expired
from the spool. When the message is (re)inserted into the scheduled queue, if
the next due time that is computed would be equal or later than the `expires`
time, the message will be expired, removed from spool, and an `Expiration`
record logged.

If you do not specify an `expires` field, the
[max_age](../kumo/make_queue_config/max_age.md) for the containing queue will
be used as normal.

## dow

String.

Specifies the comma separated list of days of the week on which delivery will
be permitted.  Days can be three-letter prefixes or the full English day names.
For example both `"Mon,Tue,Wed,Thu,Fri"` and
`"Monday,Tuesday,Wednesday,Thursday,Friday"` are acceptable ways to indicate
working week days.

## tz

String.

Specifies the name of the timezone in which to interpret the scheduling
constraints.  The timezone name must be a name from the [IANA Time Zone
Data](https://www.iana.org/time-zones) such as `"America/Phoenix"`.  Short
forms like `"PST"` have ambiguous interpretations and are NOT accepted.

## start

String.

Specifies the time of day in `"HH:MM:SS"` form of the start of an acceptable
delivery window.  The time is interpreted in the timezone specified by the
`tz` field.

## end

String.

Specifies the time of day in `"HH:MM:SS"` form of the end of an acceptable
delivery window.  The time is interpreted in the timezone specified by the
`tz` field.

