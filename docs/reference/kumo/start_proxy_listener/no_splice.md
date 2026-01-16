# no_splice

On Linux, the proxy server uses `splice(2)` for efficient zero-copy data
transfer between the client and remote connections. This can significantly
reduce CPU usage for high-throughput proxy scenarios.

Set this to `true` to disable splice and use regular userspace copying instead.
This may be useful for debugging or if you encounter issues with splice on
your system.

The default is `false` (splice is enabled on Linux).

Note: splice(2) is only used for plain TCP connections. TLS connections
always use regular copying because the data must be decrypted/encrypted
in userspace.

```lua
kumo.start_proxy_listener {
  listen = '0.0.0.0:1080',
  no_splice = true,
}
```

