# `kumo.configure_bounce_classifier {PARAMS}`

Configures the bounce classifier. The purpose of the classifier
is to attempt to digest complex and wide-ranging responses into
a smaller set of categories to help inform the sender how best
to respond and react to a delivery failure.

This function should be called only from inside your [init](../events/init.md)
event handler.

The following classifications are pre-defined:

|Label | Meaning|
|------|--------|
|InvalidRecipient|The recipient is invalid|
|DNSFailure|The message bounced due to a DNS failure.|
|SpamBlock|The message was blocked by the receiver as coming from a known spam source.|
|SpamContent|The message was blocked by the receiver as spam|
|ProhibitedAttachment|The message was blocked by the receiver because it contained an attachment|
|RelayDenied|The message was blocked by the receiver because relaying is not allowed.|
|AutoReply|The message is an auto-reply/vacation mail.|
|TransientFailure|Message transmission has been temporarily delayed.|
|Subscribe|The message is a subscribe request.|
|Unsubscribe|The message is an unsubscribe request.|
|ChallengeResponse|The message is a challenge-response probe.|
|BadConfiguration|messages rejected due to configuration issues with remote host|5.X.X error|
|BadConnection|messages bounced due to bad connection issues with remote host|4.X.X error|
|BadDomain|messages bounced due to invalid or non-existing domains|5.X.X error|
|ContentRelated|messages refused or blocked due to content related reasons|5.X.X error|
|InactiveMailbox|messages rejected due to expired|inactive, or disabled recipient addresses, 5.X.X error|
|InvalidSender|messages bounced due to invalid DNS or MX entry for sending domain|
|MessageExpired|messages bounced due to not being delivered before the bounce-after|4.X.X error|
|NoAnswerFromHost|messages bounces due to receiving no response from remote host after connecting|4.X.X or 5.X.X error|
|PolicyRelated|messages refused or blocked due to general policy reasons|5.X.X error|
|ProtocolErrors|messages rejected due to SMTP protocol syntax or sequence errors|5.X.X error|
|QuotaIssues|messages rejected or blocked due to mailbox quota issues|4.X.X or 5.X.X error|
|RelayingIssues|messages refused or blocked due to remote mail server relaying issues|5.X.X error|
|RoutingErrors|messages bounced due to mail routing issues for recipient domain|5.X.X error|
|SpamRelated|messages refused or blocked due to spam related reasons|5.X.X error|
|VirusRelated|messages refused or blocked due to virus related reasons|5.X.X error|
|AuthenticationFailed|authentication policy was not met|
|Uncategorized|messages rejected due to other reasons|4.X.X or 5.X.X error|

{{since('dev', indent=True)}}
    It is now possible to define your own classification labels. You can do so
    simply by using whatever label you like.  It is more efficient (uses less memory)
    to use one of the predefined codes.

The classifier must be configured with a set of rules files
that provide mappings from a set of regular expressions to
the available classification codes.

`kumo.configure_bounce_classifier` will compile the merged
set of files and rules into an efficient regex set that can
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

You may create and maintain your own classifications and add them to the list
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
