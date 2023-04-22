# Configuring Feedback Loop Processing

Feedback Loops are provided by several mailbox providers, including AOL, Hotmail, Comcast, and Yahoo! as a method for informing senders regarding which messages are receiving spam complaints.

The mailbox providers send registered senders formatted abuse report messages to a pre-configured address in ARF format, but those messages typically do not include information that can help senders suppress future sends to the recipient that reported the message. KumoMTA can not only process and log ARF messages, but it can also inject tracking headers into the message that it can later decode to preserve recipient data needed for unsubscribing recipients that have reported messages as spam.

## Configuring Tracking Headers

By default, KumoMTA will include a supplemental tracking header that will be extracted as part of the ARF message processing. This setting is controlled by the *supplemental_header* option in the **kumo.start_esmtp_listener** function. Additional metadata can be preserved by listing the metadata keys desired in the *include_meta_names* argument.

```lua
kumo.start_esmtp_listener {
  -- ..
  trace_headers = {
    -- this is the default: add the supplemental header
    supplemental_header = true,

    -- this is the default: the name of the supplemental header
    header_name = 'X-KumoRef',

    include_meta_names = { 'tenant', 'campaign' },
  },
}
```

See the trace headers section of the [start_esmtp_listener](../../reference/kumo/start_esmtp_listener.md#trace_headers) section of the reference manual for more information.

## Configuring ARF Domains

For KumoMTA to process inbound messages as ARF, the inbound receiving domain must be configured as a candidate for ARF processing. This is done as part of [Configuring Inbound and Relay Domains](./domains.md) in the [start_esmtp_listener](../../reference/kumo/start_esmtp_listener.md#domains) function call:

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',

  -- override the default set of relay hosts
  relay_hosts = { '127.0.0.1', '192.168.1.0/24' },

  -- Configure the domains that are allowed for outbound & inbound relay,
  -- Out-Of-Band bounces, and Feedback Loop Reports.
  -- See https://docs.kumomta.com/userguide/configuration/domains/
  domains = {
    ['fbl.examplecorp.com'] = {
      -- accept and log ARF feedback reports sent to fbl.examplecorp.com
      log_arf = true,
    },
  },
}
```

The preceding example designates that messages injected from remote hosts destined for fbl.examplecorp.com will be accepted and then processed as ARF abuse report messages.

## FBL Message Logs

All feedback loop messages are logged to the destination configured in the [configure_local_logs](../../reference/kumo/configure_local_logs.md) function, using the `Feedback` type.

The format of a Feedback loop message log entry is as follows:

```json
{
    "type": "Feedback",
    "feedback_report": {
        "feedback_type": "abuse",
        "user_agent": "SomeGenerator/1.0",
        "version": 1,
        "arrival_date": "2005-03-08T18:00:00Z",
        "incidents": nil,
        "original_envelope_id": nil,
        "original_mail_from": "<somesender@example.net>",
        "reporting_mta": {
            "mta_type": "dns",
            "name": "mail.example.com",
        },
        "source_ip": "192.0.2.1",
        "authentication_results": [
            "mail.example.com; spf=fail smtp.mail=somesender@example.com",
        ],
        "original_rcpto_to": [
            "<user@example.com>",
        ],
        "reported_domain": [
            "example.net",
        ],
        "reported_uri": [
            "http://example.net/earn_money.html",
            "mailto:user@example.com",
        ],

        // any fields found in the report that do not correspond to
        // those defined by RFC 5965 are collected into this
        // extensions field
        "extensions": {
            "removal-recipient": [
                "user@example.com",
            ],
        },

        // The original message or message headers, if provided in
        // the report
        "original_message": "From: <somesender@example.net>
Received: from mailserver.example.net (mailserver.example.net
    [192.0.2.1]) by example.com with ESMTP id M63d4137594e46;
    Tue, 08 Mar 2005 14:00:00 -0400
X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoidGVzdEBleGFtcGxlLmNvbSJ9
To: <Undisclosed Recipients>
Subject: Earn money
MIME-Version: 1.0
Content-type: text/plain
Message-ID: 8787KJKJ3K4J3K4J3K4J3.mail@example.net
Date: Thu, 02 Sep 2004 12:31:03 -0500

Spam Spam Spam
Spam Spam Spam
Spam Spam Spam
Spam Spam Spam
",

        // if original_message is present, and a kumo-style trace
        // header was decoded from it, then this holds the decoded
        // trace information
        "supplemental_trace": {
            "recipient": "test@example.com",
        },
    }
}
```