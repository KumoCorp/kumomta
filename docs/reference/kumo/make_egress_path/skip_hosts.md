# skip_hosts

A CIDR list of hosts that should be removed from the list of hosts returned
when resolving the MX for the destination domain.

This can be used for example to skip a host that is experiencing issues.

If all of the hosts returned for an MX are filtered out by `skip_hosts` then
the complete contents of the ready queue will be immediately processed.  The
behavior depends on the version of kumod:

|Behavior|Since|
|--------|-----|
|Transiently Failed with a `451 4.4.4 KumoMTA internal: MX consisted solely of hosts on the skip_hosts list` status|{{since('dev', inline=True)}}|
|Permanently Failed with a `550 5.4.4 MX consisted solely of hosts on the skip_hosts list` status|All earlier versions|

## Skipping IPv6 Addresses

When performing MX resolution, KumoMTA will always resolve both the IPv4 and
IPv6 addresses as part of its connection plan.  If your infrastructure cannot
or for policy reasons, should not use IPv6 you can set:

```lua
kumo.make_egress_path {
  -- Don't use IPv6 for deliveries
  skip_hosts = { '::/0' },
}
```

If you are using the shaping helper, you can express that in your `shaping.toml`:

```toml
[default]
skip_hosts = ["::/0"]
```
