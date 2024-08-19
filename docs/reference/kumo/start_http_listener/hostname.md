# hostname

Specifies the hostname to use when configuring TLS.
The default, if unspecified, is to use the hostname of the local machine.

```lua
kumo.start_http_listener {
  -- ..
  hostname = 'mail.example.com',
}
```


