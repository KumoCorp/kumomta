# tls_required_client_ca

{{since('2025.10.06-5ec871ab')}}

Specify the path to a TLS certificate file to use to verify a client
certificate presented by a client when it issues `STARTTLS`.

The value is an optional [KeySource](../../keysource.md).

```lua
kumo.start_esmtp_listener {
  -- ..
  tls_required_client_ca = '/path/to/client-cert.pem',
}
```

If `tls_required_client_ca` is configured, and a client presents a TLS
certificate, if that client certificate was issued by any of the certificate
authorities contained in the `tls_required_client_ca` PEM file, then the client
certificate is considered to be verified and the `tls_peer_subject_name` meta
value will be set in the connection context and will also get logged in any
associated `Reception` log that may be produced after that point.

If no client certificate was provided, or the client certificate doesn't
verify as being issued by any of the permitted authorities, then the
`tls_peer_subject_name` meta value will be left unassigned.

