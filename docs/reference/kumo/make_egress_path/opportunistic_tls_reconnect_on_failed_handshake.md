# opportunistic_tls_reconnect_on_failed_handshake

{{since('2025.01.23-7273d2bc')}}

When set to `true` (the default is `false`), a TLS handshake failure or a failed
EHLO after the handshake causes `Opportunistic` or `OpportunisticInsecure` to
retry the same address on a fresh plaintext connection, rather than moving on
to the next address in the connection plan. An effective `Required` or
`RequiredInsecure` policy does not permit this fallback.

{{since('dev', inline=True)}}
`OpportunisticInsecure` already retries once on a fresh plaintext connection
after a handshake failure, even when this option is `false`. This option also
permits its retry after a failed post-handshake EHLO, and supplies broken-TLS
memory when [remember_broken_tls](remember_broken_tls.md) is unset. The implicit
`OpportunisticInsecure` handshake retry alone does not enable that memory. With
this option disabled, a `530` reply mentioning STARTTLS at MAIL FROM on the
implicit retry causes KumoMTA to close the connection and try the next address in
the connection plan, rather than bounce the message. If there are no remaining
addresses, the message defers. Enabling this option preserves normal SMTP
transaction rejection handling.

A rejected STARTTLS command, handshake timeout, or TLS setup error is not a
trigger for this option.

