# enable_rset

{{since('dev')}}

When set to `true` (the default is `true`), then kumo will issue an `RSET`
SMTP command in between each message send on a connection.

The purpose of the `RSET` is a defensive measure, in case something unexpected
and unaccounted for has left the connection in an unknown or undesirable
state; it is used as a kind of "rollback" operation.

Disabling the use of `RSET` can be a micro-optimization to improve the
efficiency of a connection, but when pipelining is in use, it will really
be a marginal difference.

Perhaps the biggest reason for considering disabling this option is that
certain Postfix load-shedding configurations will penalize the use of RSET,
because it is considered to be a "junk" command.  If you see a trace like
this:

```
> RSET
> MAIL FROM:<sender@example.com>
> RCPT TO:<user@postfix.local>
< 250 2.0.0 Ok
< 421 4.7.0 mail.postfix.local Error: too many errors
> QUIT
```

then postfix on the destination is configured to penalize RSET and you may wish
to disable its use.

