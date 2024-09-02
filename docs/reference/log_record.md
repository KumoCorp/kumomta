## Log Record

The log record is a JSON object with the following shape:

```json
{
    // The record type; can be one of "Reception", "Delivery",
    // "Bounce", "TransientFailure", "Expiration", "AdminBounce",
    // "OOB" or "Feedback"
    "type": "Delivery",

    // The message spool id; corresponds to the value returned by
    // message:id()
    "id": "1d98076abbbc11ed940250ebf67f93bd",

    // The envelope sender
    "sender": "user@sender.example.com",

    // The envelope recipient
    "recipient": "user@recipient.example.com",

    // Which named queue the message was associaed with
    "queue": "campaign:tenant@domain",

    // Which MX site the message was being delivered to.
    // Empty string for Reception records.
    "site": "source2->(alt1|alt2|alt3|alt4)?.gmail-smtp-in.l.google.com.",

    // The size of the message payload, in bytes
    "size": 1047,

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
        // This field can be nil/absent in some cases, particularly
        // for Rejection records for incoming SMTP in older
        // versions of kumomta.
        "command": "."
    },

    // Information about the peer in the communication. This is either
    // the submitter or the receiver, depending on the record type
    "peer_address": {
        // When delivering, this is the name from the MX record.
        // When receiving, this is the EHLO/HELO string sent by
        // the sender
        "name": "gmail-smtp-in.l.google.com.",
        "addr": "142.251.2.27"
    },

    // The time at which this record was generated, expressed
    // as a unix timestamp: seconds since the unix epoch
    "timestamp": 1678069691,

    // The time at which the message was received, expressed
    // as a unix timestamp: seconds since the unix epoch
    "created": 1678069691,

    // The number of delivery attempts.
    "num_attempts": 0,

    // the classification assigned by the bounce classifier,
    // or Uncategorized if unknown or the classifier is not configured.
    "bounce_classification": "Uncategorized",

    // The name of the egress pool used as the source for the delivery
    "egress_pool": "pool0",

    // The name of the selected egress source (a member of the egress pool)
    // used for the delivery
    "egress_source": "source2",

    // For SMTP delivery, the source address (and port) that was used.
    // {{since('2024.09.02-c5476b89', inline=True)}}
    "source_address": {
        // The source address. The port number may be unknown and reported
        // as zero when using a proxy protocol.
        "address": "10.0.0.1:53210",
        // If a proxy protocol was used, this field will be
        // set to its name. It may be null/not set for no proxy,
        // "haproxy" or "socks5".
        "protocol": "socks5",
        // If a proxy protocol was used, this field will be
        // set to the proxy server address. It will be null/not set
        // when no proxy was used.
        "server": "192.168.1.1:5000"
    },

    // when "type" == "Feedback", holds the parsed feedback report
    "feedback_report": null,

    // holds the values of the list of meta fields from the logger
    // configuration
    "meta": {},

    // holds the values of the list of message headers from the logger
    // configuration
    "headers": {},

    // The protocol used to deliver, or attempt to deliver, this message.
    // May be null or unset for expirations or administrative bounces
    // or other similar situations.
    // "ESMTP" for SMTP, "Maildir" for maildir and "Lua" for a lua delivery
    // mechanism.
    "delivery_protocol": "ESMTP",

    // The protocol used to receive the message
    // "ESMTP" for SMTP, "HTTP" for the HTTP injection API, "LogRecord"
    // for messages captured via `configure_log_hook`.
    // This information is also stored in the message meta key named
    // "reception_protocol".
    "reception_protocol": "ESMTP",

    // The node uuid. This identifies the node independently from its
    // IP address or other characteristics present in this log record.
    "nodeid": "557f3ad4-2c8c-11ee-976e-782d7e12e173",

    // Information about TLS used for outgoing SMTP, if applicable.
    // These fields are present in dev builds only:
    "tls_cipher": "TLS_AES_256_GCM_SHA384",
    "tls_protocol_version": "TLSv1.3",
    "tls_peer_subject_name": ["C=US","ST=CA","L=SanFrancisco","O=Fort-Funston",
                              "OU=MyOrganizationalUnit","CN=do.havedane.net",
                              "name=EasyRSA","emailAddress=me@myhost.mydomain"]}
}
```

## Record Types

The following record types are defined:

* `"Reception"` - logging the reception of a message via SMTP or via
  the HTTP injection API
* `"Delivery"` - logging the successful delivery of a message via SMTP
* `"Bounce"` - logging a permanent failure response and end of delivery
  attempts for the message.
* `"TransientFailure"` - logging a transient failure when attempting delivery
* `"Expiration"` - logged when the message exceeds the configured maximum
  lifetime in the queue.
* `"AdminBounce"` - logged when an administrator uses the `/api/admin/bounce`
  API to fail message(s).
* `"OOB"` - when receiving an out of band bounce with an attached RFC3464
  delivery status report, the parsed report is used to synthesize an OOB
  record for each recipient in the report.
* `"Feedback"` - when receiving an ARF feedback report, instead of logging
  a `"Reception"`, a `"Feedback"` record is logged instead with the report
  contents parsed out and made available in the `feedback_report` field.
* `"Rejection"` - logging a 4xx or 5xx response generated by KumoMTA
  in response to an incoming SMTP command. {{since('2024.06.10-84e84b89', inline=True)}}

## Feedback Report

ARF feedback reports are parsed into a JSON object that has the following
structure.  The fields of the `feedback_report` correspond to those defined
by [RFC 5965](https://www.rfc-editor.org/rfc/rfc5965).

See also [trace_headers](kumo/start_esmtp_listener/trace_headers.md) for information
about the `supplemental_trace` field.

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

