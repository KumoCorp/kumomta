# banner

Customize the banner that is returned to clients when they first connect.
The configured hostname will be automatically prepended to this text, so
you should not include a hostname.

```lua
kumo.start_esmtp_listener {
  -- ..
  banner = 'Welcome to KumoMTA!',
}
```


