# refresh_interval

{{since('2024.09.02-c5476b89')}}

Specifies how long this particular egress path object should be considered to
be current when the [refresh_strategy](refresh_strategy.md) is set to `Ttl`.

The default value for this is `"60 seconds"`.  This option accepts any duration
string value.
