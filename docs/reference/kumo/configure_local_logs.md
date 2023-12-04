# `kumo.configure_local_logs {PARAMS}`

Enables local logging of reception and delivery events to the specified
`log_dir` directory.

Logs are written as zstd-compressed log file segments under the specified
directory.  Each line of the file is a JSON object holding information about
a reception or delivery related event.

This function should be called only from inside your [init](../events/init.md)
event handler.

```lua
kumo.on('init', function()
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs',
  }
end)
```

PARAMS is a lua table that can accept the keys listed below:

## back_pressure

Maximum number of outstanding items to be logged before
the submission will block; helps to avoid runaway issues
spiralling out of control.

```lua
kumo.configure_local_logs {
  -- ..
  back_pressure = 128000,
}
```

## compression_level

Specifies the level of *zstd* compression that should be used.  Compression
cannot be disabled.

Specifying `0` uses the zstd default compression level, which is `3` at the
time of writing.

Possible values are `1` (cheapest, lightest) through to `21`.

```lua
kumo.configure_local_logs {
  -- ..
  compression_level = 3,
}
```

## filter_event

{{since('2023.11.28-b5252a41')}}

Optional string. If provided, specifies the name of an event that should
be triggered to decide whether logs for a given message should be included
in this instance of local file logging.

The event will be passed the message that is being considered for logging
purposes.  The goal of the event is to return `true` if logging should
proceed or `false` otherwise.

You may access the message metadata to make that decision.

```lua
kumo.on('init', function()
  -- We only want logs for messages accepted via SMTP to land here
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs-smtp',
    filter_event = 'should_log_to_smtp_logs',
  }

  -- We want all other logs to land here
  kumo.configure_local_logs {
    log_dir = '/var/log/kumo-logs-other',
    filter_event = 'should_log_to_other',
  }
end)

kumo.on('should_log_to_smtp_logs', function(msg)
  return msg:get_meta 'reception_protocol' == 'ESMTP'
end)

kumo.on('should_log_to_other', function(msg)
  return msg:get_meta 'reception_protocol' ~= 'ESMTP'
end)
```

## headers

Specify a list of message headers to include in the logs. The default is
empty.

```lua
kumo.configure_local_logs {
  -- ..
  headers = { 'Subject' },
}
```

{{since('dev', indent=True)}}
    Header names can now use simple wildcard suffixes; if the last character
    of the header name is `*` then it will match any string with that prefix.
    For example `"X-*"` will match any header names that start with `"X-"`.

## log_dir

Specifies the directory into which log file segments will be written.
This is a required key; there is no default value.

```lua
kumo.configure_local_logs {
  -- ..
  log_dir = '/var/log/kumo-logs',
}
```

## max_file_size

Specify how many uncompressed bytes to allow per file segment. When this number
is exceeded, the current segment is finished and a new segment is created.

Segments are created using the current time in the form `YYYYMMDD-HHMMSS` so that
it is easy to sort the segments in chronological order.

The default value is ~1GB of uncompressed data, which compresses down to around
50MB of data per segment with the default compression settings.

```lua
kumo.configure_local_logs {
  -- ..
  max_file_size = 1000000000,
}
```

## max_segment_duration

Specify the maximum time period for a file segment.  The default is unlimited.

If you set this to `"1min"`, you indicate that any given file should cover a
time period of 1 minute in duration; when that time period elapses, the current
file segment, if any, will be flushed and closed and any subsequent events will
cause a new file segment to be created.

```lua
kumo.configure_local_logs {
  -- ..
  max_segment_duration = '5 minutes',
}
```

## meta

Specify a list of message meta fields to include in the logs. The default is
empty.

```lua
kumo.configure_local_logs {
  -- ..
  meta = { 'my-meta-1' },
}
```

## per_record

Allows configuring per-record type logging.

{% raw %}
```lua
kumo.configure_local_logs {
  per_record = {
    Reception = {
      -- use names like "20230306-022811_recv" for reception logs
      suffix = '_recv',
    },

    Delivery = {
      -- put delivery logs in a different directory
      log_dir = '/var/log/kumo/delivery',
    },

    TransientFailure = {
      -- Don't log transient failures
      enable = false,
    },

    Bounce = {
      -- Instead of logging the json record, evaluate this
      -- template string and log the result.
      template = [[Bounce! id={{ id }}, from={{ sender }} code={{ code }} age={{ timestamp - created }}]],
    },

    -- For any record type not explicitly listed, apply these settings.
    -- This effectively turns off all other log records
    Any = {
      enable = false,
    },
  },
}
```
{% endraw %}

The keys of the `per_record` table must correspond to one of the
record types listed below, or the special `Any` key which can be used
to match any record type that was not explicitly listed.  The values of
the `per_record` table are `LogRecordParams` have the following fields
and values:

* `suffix` - a string to append to the generated segment file name.
  For example, `suffix = '.csv'` will generate names like `20230306-022811.csv`.
* `log_dir` - specify an alternative log directory for this type
* `enable` - defaults to `true`. If you set it to `false`, records of this
  type will not be logged
* `segment_header` - ({{since('2023.11.28-b5252a41', inline=True)}}) text that will be written
  out to each newly opened segment file. Useful for emitting eg: a CSV header
  line.
* `template` - the template to use to format the log line. Continue reading
  below for more information.

The [Mini Jinja](https://docs.rs/minijinja/latest/minijinja/) templating engine
is used to evalute logging templates.  The full supported syntax is [documented
here](https://docs.rs/minijinja/latest/minijinja/syntax/index.html).

The JSON log record fields shown in the section below are assigned as template
variables, so using `{{ id }}` in your log template will be substituted with
the `id` field from the log record section below.

{{since('2023.11.28-b5252a41', indent=True)}}
    You may now use `log_record` to reference the entire log record,
    which is useful if you want to replicate the default json representation
    of the log record for an individual record type.

    You might wish to use something like the following:

    {% raw %}
    ```lua
    per_record = {
        Feedback = {
            template = [[{{ log_record | tojson }}]]
        }
    }
    ```
    {% endraw %}

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

## Feedback Report

ARF feedback reports are parsed into a JSON object that has the following
structure.  The fields of the `feedback_report` correspond to those defined
by [RFC 5965](https://www.rfc-editor.org/rfc/rfc5965).

See also [trace_headers](start_esmtp_listener.md#trace_headers) for information
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
