# openssl_options

{{since('2024.09.02-c5476b89')}}

Optional string.

If set, then the value will be used to configure openssl option flags.

OpenSSL is used as described under the
[tls_prefer_openssl](tls_prefer_openssl.md) option.

The format of the string is the set of possible option names separated by `|` characters.

Option names are:

* `ALL` - A “reasonable default” set of options which enables compatibility flags.
* `NO_QUERY_MTU` - Do not query the MTU.  Only affects DTLS connections.
* `COOKIE_EXCHANGE` - Enables Cookie Exchange as described in [RFC 4347 Section
  4.2.1](https://tools.ietf.org/html/rfc4347#section-4.2.1).  Only affects DTLS
  connections.
* `NO_TICKET` - Disables the use of session tickets for session resumption.
* `NO_SESSION_RESUMPTION_ON_RENEGOTIATION` - Always start a new session when performing a renegotiation on the server side.
* `NO_COMPRESSION` - Disables the use of TLS compression.
* `ALLOW_UNSAFE_LEGACY_RENEGOTIATION` - Allow legacy insecure renegotiation with servers or clients that do not support secure renegotiation.
* `SINGLE_ECDH_USE` - Creates a new key for each session when using ECDHE.  This is always enabled in OpenSSL 1.1.0.
* `SINGLE_DH_USE` - Creates a new key for each session when using DHE.  This is always enabled in OpenSSL 1.1.0.
* `TLS_ROLLBACK_BUG` - Disables version rollback attach detection.
* `NO_SSLV2` - Disables the use of SSLv2.
* `NO_SSLV3` - Disables the use of SSLv3.
* `NO_TLSV1` - Disables the use of TLSv1.0.
* `NO_TLSV1_1` - Disables the use of TLSv1.1.
* `NO_TLSV1_2` - Disables the use of TLSv1.2.
* `NO_TLSV1_3` - Disables the use of TLSv1.3.
* `NO_DTLSV1` - Disables the use of DTLSv1.0.
* `NO_DTLSV1_2` - Disables the use of DTLSv1.2.
* `NO_RENEGOTIATION` - Disallow all renegotiation in TLSv1.2 and earlier.
* `ENABLE_MIDDLEBOX_COMPAT` - Enable TLSv1.3 Compatibility mode.  Requires
  OpenSSL 1.1.1 or newer. This is on by default in 1.1.1, but a future version
  may have this disabled by default.

<!--
* `CIPHER_SERVER_PREFERENCE` - Use the server’s preferences rather than the
  client’s when selecting a cipher.  This has no effect on the client side;
  this option is included here for the sake of completeness.
* `PRIORITIZE_CHACHA` - Prioritize ChaCha ciphers when preferred by clients. Applies to server only
-->


