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

## tls_private_key

Specify the path to the TLS private key file that corresponds to the `tls_certificate`.

The default, if unspecified, is to dynamically allocate a self-signed certificate.

```lua
kumo.start_http_listener {
  -- ..
  tls_private_key = '/path/to/key.pem',
}
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
