# prohibited_hosts

A CIDR list of hosts that should be considered "poisonous", for example, because
they might cause a mail loop.

When resolving the hosts for the destination MX, if any of the hosts are
present in the `prohibited_hosts` list then the ready queue will be immediately
failed with a `550 5.4.4` status.


