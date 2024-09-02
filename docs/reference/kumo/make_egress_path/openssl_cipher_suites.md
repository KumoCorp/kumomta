# openssl_cipher_suites

{{since('2024.09.02-c5476b89')}}

Optional string.

If set, then the value will be used to configure the set of ciphers used by
OpenSSL for TLS protocol version 1.3.

OpenSSL is used as described under the
[tls_prefer_openssl](tls_prefer_openssl.md) option.

The format consists of TLSv1.3 cipher suite names separated by `:` characters
in order of preference.


