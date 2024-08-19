# max_messages_per_connection

Specifies the maximum number of consecutive `MAIL FROM` commands that can be
issued for a given SMTP connection.  When the limit is reached, transient
failures will be returned to those additional `MAIL FROM` commands.

```lua
kumo.start_esmtp_listener {
  max_messages_per_connection = 10000,
}
```


