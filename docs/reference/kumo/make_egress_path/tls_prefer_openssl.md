# tls_prefer_openssl

{{since('2024.09.02-c5476b89')}}

Optional boolean. Defaults to `false`.

When set to `true`, this pathway will prefer to use OpenSSL as the TLS
implementation.

When set to `false`, this pathway will prefer to use rustls as the TLS
implementation, unless DANE is enabled and TLSA records are present, in which
case OpenSSL will be used.


