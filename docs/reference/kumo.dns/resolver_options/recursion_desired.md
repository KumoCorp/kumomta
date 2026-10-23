# recursion_desired

{{since('2025.03.19-1d3f1f67')}}

Boolean. Whether to set the RD (Recursion Desired) bit on outgoing queries.
You want this on when talking to a recursive resolver, and off when talking
directly to an authoritative server. Defaults to `true`.
