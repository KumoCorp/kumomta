---
description: "Skip IPv6 MX hosts for outbound SMTP in KumoMTA with the ip_lookup_strategy option, set per egress path or as a default in your shaping.toml."
---

# How do I skip IPv6 MX hosts for outbound SMTP?

Depending on the version of KumoMTA, you have two options:

## ip_lookup_strategy

You can use the [ip_lookup_strategy](../reference/kumo/make_egress_path/ip_lookup_strategy.md)
option to prevent resolving `IPv6` addresses for MX hosts.

You can set this as the default in your `shaping.toml` if you wish:

```toml
["default"]
ip_lookup_strategy = "Ipv4Only"
```

## skip_hosts

You can use the [skip_hosts](../reference/kumo/make_egress_path/skip_hosts.md)
option to skip all IPv6 hosts by using the IPv6 CIDR `::/0` which matches all
possible IPv6 addresses.

You can set this as the default in your `shaping.toml` if you wish:

```toml
["default"]
skip_hosts = ["::/0"]
```

