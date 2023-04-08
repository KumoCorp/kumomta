# `kumo.configure_bounce_classifier {PARAMS}`

Configures the bounce classifier. The purpose of the classifier
is to attempt to digest complex and wide-ranging responses into
a smaller set of categories to help inform the sender how best
to respond and react to a delivery failure.

This function should be called only from inside your [init](../events/init.md)
event handler.

!!!
    The precise set of classifications are not yet finalized so are
    not reproduced here. They can be found in the `BounceClass` enum
    in `kumomta/crates/bounce-classify/src/lib.rs`

The classifier must be configured with a set of rules files
that provide mappings from a set of regular expressions to
the available clasification codes.

`kumo.configure_bounce_classifier` will compile the merged
set of files and rules into an efficient regexset that can
quickly match the rule to the classification code.

Once the classifier has been configured via this function,
the logging functions will automatically call into it to
populate the `bounce_classification` field.

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

The `iana.toml` file is provided with rules that map from
[IANA defined status
codes](https://www.iana.org/assignments/smtp-enhanced-status-codes/smtp-enhanced-status-codes.xhtml)
to an appropriate bounce class.

You may create and maintain your own classifications and add then to the list
of files.

Here's an excerpt of the `iana.toml`:

```toml
# This file contains rules that match SMTP ENHANCEDSTATUSCODES
# codes as defined in the IANA registry:
# https://www.iana.org/assignments/smtp-enhanced-status-codes/smtp-enhanced-status-codes.xhtml
# to bounce classifications.
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
InactiveMailbox = [
  "^(451|550) [45]\\.1\\.[6] ",
  "^[45]\\d{2} [45]\\.2\\.1 ",
  "^525 [45]\\.7\\.13 ", # User account disabled
]
InvalidSender = [
  "^(451|550) [45]\\.1\\.[78] ",
  "^\\d{3} [45]\\.7\\.27 ", # Send address has NULL MX
]
QuotaIssues = [
  "^552 [45]\\.2\\.2 ",
  "^552 [45]\\.2\\.3 ",
  "^452 [45]\\.3\\.1 ", # Mail System Full
  "^55[24] [45]\\.3\\.4 ", # Message too large for system
]
```
