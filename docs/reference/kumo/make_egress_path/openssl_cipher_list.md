# openssl_cipher_list

{{since('dev')}}

Optional string.

If set, then the value will be used to configure the set of ciphers used by
OpenSSL for TLS protocol version lower than 1.3.

OpenSSL is used as described under the
[tls_prefer_openssl](#tls_prefer_openssl) option above.

The format of the string is [discussed in the OpenSSL ciphers
documentation](https://www.openssl.org/docs/man1.1.1/man1/ciphers.html)


