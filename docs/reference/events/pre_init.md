# `kumo.on('pre_init', FUNCTION)`

{{since('2023.11.28-b5252a41')}}

The `pre_init` event is triggered once when the `kumod` process initializes,
prior to triggering the [init](init.md) event.

`pre_init` can be registered multiple times.

The intended purpose of this event is to be used by lua helper modules to aid
in building up optional modular functionality.

