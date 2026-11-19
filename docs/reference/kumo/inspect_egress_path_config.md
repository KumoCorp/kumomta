# kumo.inspect_egress_path_config

```lua
local view = kumo.inspect_egress_path_config(path_config)
```

{{since('dev')}}

Returns an egress path configuration table for display without loading its [key sources](../keysource.md). If `smtp_auth_plain_password`, `tls_private_key`, or `tls_certificate.vault_token` is explicitly configured, its value is shown as `"[REDACTED]"`; unset fields stay unset. The rest of the certificate source remains visible, including callback arguments. Other path settings retain their values. Use the original configuration for delivery.

The `kcli inspect-ready-q` and `kcli resolve-egress-path` server responses and the `resolve-shaping-domain` script use this representation.
