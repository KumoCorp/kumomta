# trusted_hosts

Specify the hosts which are trusted to access the HTTP service.
Each item can be an IP literal or a CIDR mask.

The defaults are to allow the local host.

```lua
kumo.start_http_listener {
  -- ..
  trusted_hosts = { '127.0.0.1', '::1' },
}
```


