# rustls_cipher_suites

{{since('dev')}}

Optional array of strings.

If set, then the value will be used to configure rustls cipher suites.

Rustls is used as described under the
[tls_prefer_openssl](tls_prefer_openssl.md) option above.

The list of possible cipher suites at the time of writing is:

* `TLS13_AES_256_GCM_SHA384`
* `TLS13_AES_128_GCM_SHA256`
* `TLS13_CHACHA20_POLY1305_SHA256`
* `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`
* `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`
* `TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256`
* `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`
* `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`
* `TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256`

This list may evolve over time as new releases of kumomta are made.  You can
review the current possible list with the `tls-probe` utility:

```console
$ /opt/kumomta/sbin/tls-probe list-rustls-cipher-suites
TLS13_AES_256_GCM_SHA384
TLS13_AES_128_GCM_SHA256
TLS13_CHACHA20_POLY1305_SHA256
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
```


