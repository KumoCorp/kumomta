# line_length_hard_limit

{{since('2023.11.28-b5252a41')}}

The SMTP protocol specification defines the maximum length of a line in the
protocol.  The limit exists because there are SMTP implementations that are
simply not capable of reading longer lines.

This option sets the limit on line length that is enforced by KumoMTA. The
default matches the RFC specified limit of `998` + CRLF.  When the line length
limit is exceeded, KumoMTA will return a "line too long" error to the
client.

You can raise this limit, but doing so may allow messages to be accepted
that will be unable to be relayed to other SMTP implementations.


