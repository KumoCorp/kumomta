# kumo.uuid.new_v6()

```lua
kumo.uuid.new_v6()
```

{{since('2024.09.02-c5476b89')}}

Create a new [version 6
UUID](https://www.ietf.org/rfc/rfc9562.html#section-5.6) with the current
timestamp and the current node ID.

This is similar to version 1 UUIDs, except that it is lexicographically
sortable by timestamp.

The node ID is taken from the MAC address of the first non-loopback interface
on the system. If there are no candidate interfaces, fall back to the
`gethostid()` function, which, on most Linux systems, will attempt to load a
host id from a file on the filesystem, or if that fails, resolve the hostname
of the node to its IPv4 address using a reverse DNS lookup, and then derive
some 32-bit number from that address through unspecified means.

See [The UUID Object](index.md#the-uuid-object) for more information about the
returned UUID object.

