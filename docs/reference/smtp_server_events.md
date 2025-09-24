# SMTP Server Events

The following sequence diagram shows the ordering of the various SMTP server
Lua events that are triggered by the SMTP listener in response to SMTP commands
issued by the client.

```mermaid
sequenceDiagram
  participant C as Client
  participant S as SMTP Listener
  participant Lua

  C->>S: establish TCP connection
  S<<->>Lua: smtp_server_connection_accepted
  S<<->>Lua: smtp_server_get_dynamic_parameters
  Note right of S: Listener parameters<br>are now decided
  S->>C: 250 banner

  C->>S: EHLO example.com
  S<<->>Lua: smtp_server_ehlo
  S->>C: 250 OK

  C->>S: MAIL FROM: <sender@example.com>
  S<<->>Lua: smtp_server_mail_from
  S->>C: 250 OK

  C->>S: RCPT TO: <recip@example.com>
  S<<->>Lua: get_listener_domain(MAIL FROM)
  S<<->>Lua: get_listener_domain(RCPT TO)
  Note right of S: check relaying
  S<<->>Lua: smtp_server_rcpt_to
  S->>C: 250 OK

  C->>S: DATA
  S->>C: 354 send message
  C->>S: message / CRLF.CRLF
  S<<->>Lua: smtp_server_data
  Note right of S: split multi-recipient in batches<br>following steps are per split message
  Note right of S: add trace/supplemental headers
  S<<->>Lua: smtp_server_message_received
  S<<->>Lua: get_listener_domain(MAIL FROM)
  S<<->>Lua: get_listener_domain(RCPT TO)
  Note right of S: check relaying
  S<<->>Lua: get_queue_config
  Note right of S: insert into queue
  Note right of S: return overall status across<br>all split messages
  S->>C: 250 OK
```

!!! note
    While the diagram above shows `Lua` as a single actor, each lua event
    callout is likely to run in a distinct, separate, lua context.

Here's a list of links to the docs for the various events in the diagram above,
listed in the same sequence as the diagram:

  * [smtp_server_connection_accepted](events/smtp_server_connection_accepted.md)
  * [smtp_server_get_dynamic_parameters](events/smtp_server_get_dynamic_parameters.md)
  * [smtp_server_ehlo](events/smtp_server_ehlo.md)
  * [smtp_server_mail_from](events/smtp_server_mail_from.md)
  * [get_listener_domain](events/get_listener_domain.md)
  * [smtp_server_rcpt_to](events/smtp_server_rcpt_to.md)
  * [smtp_server_data](events/smtp_server_data.md)
  * [smtp_server_message_received](events/smtp_server_message_received.md)
  * [get_queue_config](events/get_queue_config.md)
