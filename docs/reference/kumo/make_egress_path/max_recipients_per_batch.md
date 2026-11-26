# max_recipients_per_batch

{{since('2025.12.02-67ee9e96')}}

Optional integer.  Defaults to `100`.

Specifies the maximum number of `RCPT TO` commands when sending a message that
has multiple recipients.

If a message has more than this number of recipients then each chunk will be
attempted successively in separate transactions. Those transactions will re-use
the current connection, but if the connection is broken, the excess will remain
eligible for immediate delivery which will typically continue on a new
connection.

Some servers use `552` instead of `452` when they cannot accept another
recipient in the same transaction. The SMTP client consults the
[bounce classifier](../configure_bounce_classifier.md) when a `RCPT TO` reply
has one of these codes without an enhanced status code. A `TooManyRecipients`
classification makes the refusal transient even for the first recipient.
When the classification is inconclusive, a `452` remains transient; for a
`552`, the recipient's position in the transaction matters:

| Version | First (or only) recipient | Later recipient |
|---------|---------------------------|-----------------|
| `2025.12.02-67ee9e96` through `2026.09.29-b90d8bc1` | Retry | Retry |
| {{since('dev', inline=True)}} | Bounce | Retry as a possible recipient limit |

