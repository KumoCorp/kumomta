# max_recipients_per_message

Specifies the maximum number of consecutive `RCPT TO` commands that can be
issued for a given SMTP transaction.  When the limit is reached, transient
failures will be returned to those additional `RCPT TO` commands.

```lua
kumo.start_esmtp_listener {
  max_recipients_per_message = 1024,
}
```


