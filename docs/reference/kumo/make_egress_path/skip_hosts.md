# skip_hosts

A CIDR list of hosts that should be removed from the list of hosts returned
when resolving the MX for the destination domain.

This can be used for example to skip a host that is experiencing issues.

If all of the hosts returned for an MX are filtered out by `skip_hosts` then
the ready queue will be immediately failed with a `550 5.4.4` status.


