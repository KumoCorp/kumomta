# allow_smtp_auth_plain_without_tls

Optional boolean. Defaults to `false`.

When `false`, and the connection is not using TLS, SMTP AUTH PLAIN will be
premptively failed in order to prevent the credential from being passed over
the network in clear text.

You can set this to `true` to allow sending the credential in clear text.

!!! danger
    Do not enable this option on an untrusted network, as the credential
    will then be passed in clear text and visible to anyone else on the
    network


