# tls_handshake_timeout

{{since('dev')}}

How long to wait for the TLS handshake to complete after the peer has accepted
the `STARTTLS` command. The default is `60s`.

This is distinct from [starttls_timeout](starttls_timeout.md), which only bounds
the `STARTTLS` command/response exchange. Without `tls_handshake_timeout` the
handshake negotiation itself is unbounded: a peer that accepts `STARTTLS` and
then stalls the handshake (sending nothing further) could keep the connection
open indefinitely, making no delivery progress and producing no error. That
wedged connection would hold its slot in the ready queue and prevent the
messages behind it from being attempted.

When the handshake exceeds this timeout the connection is torn down and the
message is requeued for a later attempt, allowing it to be retried (potentially
via a different egress source).
