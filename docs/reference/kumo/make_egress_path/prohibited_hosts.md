# prohibited_hosts

A CIDR list of hosts that should be considered "poisonous", for example, because
they might cause a mail loop.

When resolving the hosts for the destination MX, if any of the hosts are
present in the `prohibited_hosts` list then the ready queue will be immediately
failed with a `550 5.4.4` status.

The default value for this is:

 * `127.0.0.0/8`, the set of IPv4 loopback addresses
 * `::1`, the IPv6 loopback address

{{since('dev')}}

The default value is now:

 * `127.0.0.0/8`, the set of IPv4 loopback addresses
 * `0.0.0.0`, the IPv4 Any address
 * `::1`, the IPv6 loopback address
 * `::`, the IPv6 Any address
