# hostname

Specifies the hostname to report in the banner and other SMTP responses.
The default, if unspecified, is to use the hostname of the local machine.

```lua
kumo.start_esmtp_listener {
  -- ..
  hostname = 'mail.example.com',
}
```


