# How do I skip IPv6 MX hosts for outbound SMTP?

You can use the [skip_hosts](../reference/kumo/make_egress_path/skip_hosts.md)
option to skip all IPv6 hosts by using the IPv6 CIDR `::/0` which matches all
possible IPv6 addresses.

You can set this as the default in your `shaping.toml` if you wish:

```toml
["default"]
skip_hosts = ["::/0"]
```

