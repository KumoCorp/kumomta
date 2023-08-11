# Configuring Bounce Classification

By default, the logs will contain extensive information on the responses
provided by a remote host during a delivery attempt:

```json
// The response from the peer, if applicable
"response": {
    // the SMTP status code
    "code": 250,

    // The ENHANCEDSTATUSCODE portion of the response parsed
    // out into individual fields.
    // This one is from a "2.0.0" status code
    "enhanced_code": {
        "class": 2,
        "subject": 0,
        "detail": 0,
    },

    // the remainder of the response content
    "content": "OK ids=8a5475ccbbc611eda12250ebf67f93bd",

    // the SMTP command verb to which the response was made.
    // eg: "MAIL FROM", "RCPT TO" etc. "." isn't really a command
    // but is used to represent the response to the final ".:
    // we send to indicate the end of the message payload.
    "command": "."
},
```

This information includes the [IANA Status
Codes](https://www.iana.org/assignments/smtp-enhanced-status-codes/smtp-enhanced-status-codes.xhtml)
provided by the remote host, but there are a large number of codes that can be
interpreted in a variety of ways, and many mailbox providers use status codes
differently.

To make it easier to handle bounces, the Bounce Classifier can be configured:

```lua
kumo.on('init', function()
  kumo.configure_local_logs {
    log_dir = '/var/log/kumomta',
  }
  kumo.configure_bounce_classifier {
    files = {
      '/opt/kumomta/share/bounce_classifier/iana.toml',
    },
  }
end)
```

Once configured, the Bounce Classifier will populate the
*bounce_classification* field in the logs with the applicable category.

An example of classification rules:

```toml
[rules]
InvalidRecipient = [
  "^(451|550) [45]\\.1\\.[1234] ",
  "^45[02] [45]\\.2\\.4 ", # Mailing list expansion
  "^5\\d{2} [45]\\.7\\.17 ", # RRVS: Mailbox owner has changed
]
BadDomain = [
  "^(451|550) [45]\\.1\\.10 ", # NULL MX
  "^5\\d{2} [45]\\.7\\.18 ", # RRVS: domain owner has changed
]
```

Users can create their own classification rules file by copying the default
file, editing it, and adding the path to their custom rules file to the *files*
option in the **kumo.configure_bounce_classifier** function call. Each defined
rules file will be merged into the full ruleset.

For additional information, see the [reference manual page on bounce
classification](../../reference/kumo/configure_bounce_classifier.md).

## Configuring Out-Of-Band Bounce Processing

Not all bounces occur while the server is connected to the remote host. It is
not uncommon for a remote host to accept a message, perform further processing,
and then determine that the message should be rejected. This can be because of
user validation, AntiVirus/AntiSpam processing, or other post-connection logic.
When a remote host rejects a message after accepting it, RFCs require that a
Message Disposition Notification (MDN) message be sent to the return path
address of the message specified in the MAIL FROM command during the relay
session. See
[https://en.wikipedia.org/wiki/Bounce_message](https://en.wikipedia.org/wiki/Bounce_message)
for more information.

The KumoMTA server can process these MDN messages, but must be configured to
know which domains are candidates for OOB bounce processing.  This is most
simply accomplished using the [listener domains policy
helper](domains.md#using-the-listener_domainslua-policy-helper), with a snippet
like this:

```toml
["bounce.examplecorp.com"]
# accept and log OOB messages send to bounce.examplecorp.com
log_oob = true
```

## OOB Message Disposition After Processing

For most use cases, the desired outcome after a DSN message is processed is to
discard the message, but in some cases it can be desirable to forward the
message for further processing or storage, especially during testing and
migration.

To queue a message after processing, add `relay_to = true` to the listener
domain configuration:

```toml
["bounce.examplecorp.com"]
# accept and log OOB messages send to bounce.examplecorp.com
log_oob = true
relay_to = true
```

In addition, it should be noted that the MX record for your domain will still
be pointed at the KumoMTA instance, which means that in order to avoid a mail
loop you will need to configure routing for the domain to specify where the
message should be relayed to from the KumoMTA instance:

```lua
kumo.on('smtp_server_message_received', function(msg)
  if msg:recipient().domain == 'bounce.examplecorp.com' then
    -- Re-route the message to the intended destination
    msg:set_meta('routing_domain', '[192.168.1.100]')

    -- In earlier versions of KumoMTA, you need to set the queue
    -- this way instead
    -- msg:set_meta('queue', '[192.168.1.100]')
  end
end)
```
