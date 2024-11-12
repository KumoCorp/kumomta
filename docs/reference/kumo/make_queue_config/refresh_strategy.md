# refresh_strategy

{{since('2024.11.08-d383b033')}}

Defines the refresh strategy that should be used to determine when
this particular queue configuration object needs to be refreshed.

Possible values are:

* `"Ttl"` - the default value. Use the [refresh_interval](refresh_interval.md) value.
* `"Epoch"` - remains current until the [Configuration
  Monitoring](../../configuration.md#configuration-monitoring) system
  determines that the configuration epoch has changed.

