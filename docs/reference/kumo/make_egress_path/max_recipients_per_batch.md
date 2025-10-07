# max_recipients_per_batch

{{since('dev')}}

Optional integer.  Defaults to `100`.

Specifies the maximum number of `RCPT TO` commands when sending a message that
has multiple recipients.

If a message has more than this number of recipients then each chunk will be
attempted successively in separate transactions. Those transactions will re-use
the current connection, but if the connection is broken, the excess will remain
eligible for immediate delivery which will typically continue on a new
connection.

