# `kumo.start_http_listener { PARAMS }`

Configure and start HTTP service.

This function should be called only from inside your [init](../events/init.md)
event handler.

`PARAMS` is a lua table that can accept the keys listed below.

## hostname

Specifies the hostname to use when configuring TLS.
The default, if unspecified, is to use the hostname of the local machine.

```lua
kumo.start_http_listener {
  -- ..
  hostname = 'mail.example.com',
}
```

## listen

Specifies the local IP and port number to which the HTTP service
should bind and listen.

Use `0.0.0.0` to bind to all IPv4 addresses.

```lua
kumo.start_http_listener {
  listen = '0.0.0.0:80',
}
```

## tls_certificate

Specify the path to a TLS certificate file to use for the server identity when
*use_tls* is set to `true`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_http_listener {
  -- ..
  tls_certificate = '/path/to/cert.pem',
}
```

You may specify that the certificate be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_http_listener {
  -- ..
  tls_certificate = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.cert',

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The key must be stored as `key` (even though this is a certificate!) under the
`path` specified.  For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com.cert key=@mail.example.com.cert
```

## tls_private_key

Specify the path to the TLS private key file that corresponds to the `tls_certificate`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_http_listener {
  -- ..
  tls_private_key = '/path/to/key.pem',
}
```

You may specify that the key be loaded from a [HashiCorp Vault](https://www.hashicorp.com/products/vault):

```lua
kumo.start_http_listener {
  -- ..
  tls_private_key = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.key',

    -- Specify how to reach the vault; if you omit these,
    -- values will be read from $VAULT_ADDR and $VAULT_TOKEN

    -- vault_address = "http://127.0.0.1:8200"
    -- vault_token = "hvs.TOKENTOKENTOKEN"
  },
}
```

The key must be stored as `key` under the `path` specified.
For example, you might populate it like this:

```
$ vault kv put -mount=secret tls/mail.example.com key=@mail.example.com.key
```

## trusted_hosts

Specify the hosts which are trusted to access the HTTP service.
Each item can be an IP literal or a CIDR mask.

The defaults are to allow the local host.

```lua
kumo.start_http_listener {
  -- ..
  trusted_hosts = { '127.0.0.1', '::1' },
}
```

## use_tls

If true, the listener will start with TLS enabled and require clients to use
`https`.

## request_body_limit

{{since('2024.06.10-84e84b89')}}

Specifies the maximum acceptable size of an incoming HTTP request, in bytes.
The default is 2MB.

If an incoming request exceeds this limit, a `413 Payload Too Large` HTTP
response will be returned.
