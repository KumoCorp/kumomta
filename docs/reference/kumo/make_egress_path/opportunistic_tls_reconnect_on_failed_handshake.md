# opportunistic_tls_reconnect_on_failed_handshake

{{since('2025.01.23-7273d2bc')}}

When set to `true` (the default is `false`), if `enable_tls` is set to
`Opportunistic` or `OpportunisticInsecure`, and the TLS handshake, or the
subsequent EHLO after the TLS handshake, fails, instead of moving on to the
next address in the connection plan, we will establish a new connection to the
same address, but with `enable_tls` set to `Disabled`.

