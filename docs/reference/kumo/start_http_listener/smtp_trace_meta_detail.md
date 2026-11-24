# smtp_trace_meta_detail

{{since('dev')}}

Sets the minimum trace detail for top-level [`x_` metadata keys](../../message/set_meta.md#naming-convention-for-user-defined-keys) in `trace-smtp-server` connection events on this HTTP listener.

```lua
kumo.start_http_listener {
  listen = '127.0.0.1:8000',
  max_smtp_trace_detail = 'envelope',
  smtp_trace_meta_detail = {
    inbound_connection = {
      default_for_x_keys = 'envelope',
      keys = {
        x_policy_route = 'operational',
        x_auth_context = 'secrets',
      },
    },
  },
}
```

In this example, `operational` shows `x_policy_route`. `envelope` also shows unlisted `x_` keys. `x_auth_context` requires `secrets`, above this listener's [maximum](max_smtp_trace_detail.md).

`default_for_x_keys` defaults to `secrets`. An exact entry in `keys` takes precedence. Only names beginning with `x_` are accepted in `keys`. The level applies to the whole value, including any nested fields. These rules do not affect the copy in message metadata, callback results, or text in SMTP replies and policy errors.
