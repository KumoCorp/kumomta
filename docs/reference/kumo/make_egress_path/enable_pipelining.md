# enable_pipelining

{{since('2025.01.23-7273d2bc')}}

When set to `true` (the default is `true`), then kumo will use the SMTP
`PIPELINING` extension when it is advertised by the remote host.

When set to `false`, then `PIPELINING` will not be used even if it is advertised.

You typically *do* want to use `PIPELINING` when available, because it reduces
the protocol overhead and round-trips, resulting in lower latency sends per
message.
