# relay_hosts

Specify the hosts which are allowed to relay email via this ESMTP service.
Each item can be an IP literal or a CIDR mask. **Note** that the CIDR notation 
is strict, so that 192.168.1.0/24 is valid but 192.168.1.1/24 is not because 
that final octet isn’t valid in a /24.


The defaults are to allow relaying only from the local host:

```lua
kumo.start_esmtp_listener {
  -- ..
  relay_hosts = { '127.0.0.1', '::1' },
}
```


