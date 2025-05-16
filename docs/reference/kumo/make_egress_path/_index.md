# kumo.make_egress_path

```lua
kumo.make_egress_path { PARAMS }
```

Constructs a configuration object that specifies how traffic travelling the
path from a *source* to a *site* will behave.

This function should be called from the
[get_egress_path_config](../../events/get_egress_path_config.md) event handler to provide the
configuration for the requested site.

The following keys are possible:

## Egress Path Parameters { data-search-exclude }
