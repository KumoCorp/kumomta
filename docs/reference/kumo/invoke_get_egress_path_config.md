# kumo.invoke_get_egress_path_config

```lua
local config = kumo.invoke_get_egress_path_config(
     routing_domain, egress_source, site_name)
```

{{since('2025.01.23-7273d2bc')}}

This function triggers a call through to the
[get_egress_path_config](../events/get_egress_path_config.md) event callback(s)
that have been defined in the policy to obtain the effective configuration for
the specified combination of `routing_domain`, `egress_source` and `site_name`.

The result of that is then serialized and returned as a lua value that has the
same shape as the `PARAMS` defined for
[kumo.make_egress_path](make_egress_path/index.md).

!!! note
    The following fields do not presently round-trip back into lua
    and will be unavailable in the returned value:

      * `openssl_options`
      * `rustls_cipher_suites`

!!! danger
    Take care when using this function and its related `invoke_xxx` functions,
    as you can potentially create cross-dependent, mututally recursive,
    event callbacks that call into each other.

