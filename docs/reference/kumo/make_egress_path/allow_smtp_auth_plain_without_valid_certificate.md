# allow_smtp_auth_plain_without_valid_certificate

{{since('dev')}}

Optional boolean. Defaults to `false`.

When `false`, SMTP AUTH PLAIN will be preemptively failed if the TLS session's
peer certificate was not validated, in order to prevent the credential from
being captured by an active (man-in-the-middle) attacker. This covers sessions
where the effective [enable_tls](enable_tls.md) value is `OpportunisticInsecure`
or `RequiredInsecure`, as well as a [DANE](enable_dane.md) host whose published
TLSA records are unusable: in all of those cases the connection is encrypted but
the peer is not authenticated.

You can set this to `true` to restore the previous behavior of sending the
credential over any encrypted session, regardless of whether the certificate was
validated.

!!! danger
    Do not enable this option on an untrusted network. Over a TLS session with
    an unvalidated certificate, an active attacker can present their own
    certificate, capture the credential, and relay the session to the real
    server.

See also [allow_smtp_auth_plain_without_tls](allow_smtp_auth_plain_without_tls.md),
which governs the separate case of SMTP AUTH PLAIN over an unencrypted
connection.
