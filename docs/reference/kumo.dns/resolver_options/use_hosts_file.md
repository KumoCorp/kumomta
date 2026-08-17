# use_hosts_file

{{since('2025.03.19-1d3f1f67')}}

Controls whether the system hosts file (`/etc/hosts` on Unix-like systems)
is consulted during resolution.

{{since('2025.05.06-b29689af', inline=True)}}: this field is a string with one
of the values `Always`, `Auto`, or `Never`.

In earlier versions it is a boolean.

Defaults to `Auto`.
