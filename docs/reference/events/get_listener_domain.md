# `kumo.on('get_listener_domain', function(domain))`

This event is triggered by the SMTP server to retrieve information about
either a source or destination domain to help determine whether the message
should be accepted/logged/relayed.

When the SMTP `RCPT TO` command is issued by the client, the destination
domain is passed as the *domain* parameter to this event.

The event is expected to return a listener-domain object constructed
by a call to [kumo.make_listener_domain](../kumo/make_listener_domain.md),
or a `nil` value to indicate that there is no explicit configuration.

If none of `log_relay`, `log_oob` or `log_arf` are set to true, in the returned
listener-domain object, then the `RCPT TO` command is rejected.

Once the `DATA` stage has transmitted the message content, and after the
[smtp_server_message_received](../events/smtp_server_message_received.md) event
has been processed, and the reception logged (which is where OOB and FBL data
is parsed and logged), the recipient domain is passed to this event as the
*domain* parmater. If `relay_to` is `false` in the returned listener-domain
object, then the message will not be spooled and that will be the end of its
processing.

See [kumo.make_listener_domain](../kumo/make_listener_domain.md) for some more
examples.
