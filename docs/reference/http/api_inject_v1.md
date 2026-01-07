# `POST /api/inject/v1`

Making a POST request to this endpoint allows injecting 1 or more messages.

Both message assembly and templating are supported, and multiple recipients
and template substitutions can be passed in a single request.

The body of the post request must be a JSON object; here's a very basic
example:

```json
{
    "envelope_sender": "noreply@example.com",
    "content": "Subject: hello\n\nHello there",
    "recipients": [
        {
            "email": "recipient@example.com",
        }
    ]
}
```

The response will look something like:

```json
{
    "success_count": 1,
    "fail_count": 0,
    "failed_recipients": [],
    "errors": []
}
```

!!! note
    The `success_count` will always be reported as `0` when using `deferred_generation: true`.

## Fields

The following fields are defined for the inject request:

### content

Specifies the message content. It can either be a string value or
a JSON object describing how to build a the message.

If a simple string is provided, it must be an RFC822 compliant
message.  If template substitutions are used in the request, then
the entire RFC822 message string is used as-is for the template;
no message parsing or decoding is performed as part of template
expansion.

Alternatively the content can be specified as a JSON object as
demonstrated below.

!!! note
    Comments are used inline in the JSON objects on this page for the purposes of
    exposition in these docs, but comments are not valid in the actual request.

```json
{
    "envelope_sender": "noreply@example.com",
    "content": {
        "text_body": "This is the plain text part",
        "html_body": "<p>This is the <b>HTML</b> part</p>",
        // Optionally define an AMP email part.
        // This is available {{since('dev', inline=True)}}
        "amp_html_body": "<!doctype html><html amp4email>...</html>",
        "attachments": [
            {
                // The attachment data.
                // If the base64 field is true, this data must be encoded
                // using base64. Otherwise, it will be interpreted as UTF-8.
                "data": "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7",
                "base64": true,
                "content_type": "image/gif",
                // optional Content-ID. If specified, this attachment will
                // be added as an inline attachment and a multipart/related
                // MIME container will be generated in the message to hold
                // it and the textual content.
                "content_id": "my-image",
                // optional file name. If specified, will be added to the
                // MIME headers for the attachment.
                "file_name": "pixel.gif"
            }
        ],
        // Controls the From: header
        "from": {
            "email": "someone@example.com",
            "name": "Someone"
        },
        // Controls the Subject: header
        "subject": "This is the subject",
        // Controls the Reply-To: header
        "reply_to": {
            "email": "help@example.com",
            "name": "Help"
        },
        // Specify arbitrary additional headers
        "headers": {
            "X-Something": "Something!"
        }
    },
    "recipients": [
        {
            "email": "recipient@example.com"
        }
    ]
}
```

When building a message, template substitutions are applied to the *text_body*,
*html_body*, *amp_html_body* and *headers* fields.  Attachments are not subject
to template substitution.

### envelope_sender

The address to use as the envelope sender address when generating
the message.

It must be a string of the form *user@domain*.

### recipients

Specifies the list of recipients to which message(s) will be sent.
Each recipient is a JSON object:

```json
{
    "recipients": [
        {
            // The recipient's email address. Required.
            "email": "recipient@example.com",
            // Optional recipient name. Will be used to populate
            // template substitutions.
            "name": "Mr. Recipient",
            // Optional additional template substitutions
            "substitutions": {
                "key": "value",
            }
        }
    ]
}
```

### substitutions

Specifies a set of global substitutions to for template expansion:

```json
{
    "substitutions": {
        "campaign": "Summer Sale 2023",
    }
}
```

### deferred_spool

{{since('2024.11.08-d383b033')}}

!!! danger
    Enabling this option may result in loss of accountability for messages.
    You should satisfy yourself that your system is able to recognize and
    deal with that scenario if/when it arises.

When set to `true`, the generated message(s) will not be written to the spool
until it encounters its first transient failure.  This can improve injection
rate but introduces the risk of loss of accountability for the message if the
system were to crash before the message is delivered or written to spool, so
use with caution!

When used in conjunction with `deferred_generation`, both the queued generation
request and the messages which it produces are subject to deferred spooling.

### deferred_generation

{{since('2024.11.08-d383b033')}}

The default mode of operation is to respond to the injection request only
once every message in the request has been enqueued to the internal queue
system. This provides *back pressure* to the injection system and prevents
the service from being overwhelmed if the rate of ingress exceeds the
maximum rate of egress.

The result of this back pressure is that the latency of the injection request
depends on the load of the system.

Setting `deferred_generation: true` in the request alters the processing flow:
instead of immediately expanding the request into the desired number of
messages and queueing them up, the injection request is itself queued up and
processed asynchronously with respect to the incoming request.

This `deferred_generation` submission is typically several orders of magnitude
faster than the immediate generation mode, so it is possible to very very quickly
queue up large batches of messages this way.

The deferred generation requests are queued internally to a special queue
named `generator.kumomta.internal` that will process them by spawning each
request into the `httpinject` thread pool.

You will likely want and need to configure shaping to accomodate this queue
for best performance:

```lua
-- Locate this before any other helpers or modules that define
-- `get_egress_path_config` event handlers in order for it to take effect
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    if routing_domain == 'generator.kumomta.internal' then
      return kumo.make_egress_path {
        -- This is a good place to start, but you may want to
        -- experiment with 1/2, 3/4, or 1.5 times this to find
        -- what works best in your environment
        connection_limit = kumo.available_parallelism(),
        refresh_strategy = 'Epoch',
        max_ready = 80000,
      }
    end
  end
)
```

!!! note
    It is possible to very quickly generate millions of queued messages when
    using `deferred_generation: true`. You may wish to look into configuring
    a rate limit to constrain the system appropriately for your environment.
    [kumo.set_httpinject_recipient_rate_limit](../kumo/set_httpinject_recipient_rate_limit.md)
    can be used for this purpose.

### trace_headers

{{since('2024.11.08-d383b033')}}

Controls the addition of tracing headers to received messages.

KumoMTA can add two different headers to aid in later tracing:

* The standard `"Received"` header which captures SMTP relay hops on their path to the inbox
* A supplemental header which can be used to match feedback reports back to the
  originating mailing

Prior to triggering the
[http_message_generated](../events/http_message_generated.md)
event the standard `"Received"` header will be added to the
message.  Then, once the event completes and your policy has had the
opportunity to alter the meta data associated with the message, the
supplemental header will be added.

```json
{
  "trace_headers": {
    // this is the default: do NOT add the Received: header
    "received_header": false,

    // this is the default: add the supplemental header
    "supplemental_header": true,

    // this is the default: the name of the supplemental header
    "header_name": "X-KumoRef",

    // names of additional meta data fields
    // to include in the header. TAKE CARE! The header will be
    // base64 encoded to prevent casual introspection, but the
    // header is NOT encrypted and the values of the meta data
    // fields included here should be considered to be public.
    // The default is not to add any meta data fields, but you
    // might consider setting something like:
    // "include_meta_names": { "tenant", "campaign" },
    "include_meta_names": {},
  },
}
```

Here's an example of a supplemental header from a message:

```
X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoidGVzdEBleGFtcGxlLmNvbSJ9
```

the decoded payload contains a magic marker key as well as the recipient of the
original message:

```json
{"_@_":"\\_/","recipient":"test@example.com"}
```

Any meta data fields that were listed in `include_meta_names`, if the corresponding
meta data was set in the message, would also be captured in the decoded payload.

KumoMTA will automatically extract this supplemental trace header information
from any `X-` header that is successfully parsed and has the magic marker key
when processing the original message payload of an incoming ARF report.

### template_dialect

{{since('2025.12.02-67ee9e96')}}

It is now possible to specify which template engine will be
used for template expansion via the `template_dialect` field.
It can have one of the following values:

 * `Jinja` - this is the implied default.  The Mini Jinja
   template dialect will be parsed and evaluated.
 * `Static` - The content is treated as a static string and
   no template expansion will be performed
 * `Handlebars` - The content will be evaluated by a handlebars
   compatible template engine.


## Template Substitution

The injection API embeds the Mini Jinja templating engine, with
some supplemental extensions.  The syntax and extensions are
[documented here](../template/index.md).

For each recipient, the set of variables pre-defined in the template are:

* The set of global substitutions defined by `request.substitutions`

* The set of per-recipient substitutions, if any are defined in
  `request.recipients[].substitutions`, are overlaid and take precedence over
  any global substitutions

* The recipient `name` and `email` fields are assigned to the `"name"` and
  `"email"` variables respectively.

!!! note
    Both sets of *substitutions* can use any JSON value for the values of
    the variables; they don't have to be strings.

A very basic example of using templating:

```json
{
    "envelope_sender": "noreply@example.com",
    "content": "To: \"{{ name }}\" <{{ email }}>\nSubject: hello\n\nHello {{ name }}!",
    "recipients": [
        {
            "email": "recipient@example.com",
            "name": "John Smith"
        }
    ]
}
```

would result in an message with the following content:

```
To: "John Smith" <recipient@example.com>
Subject: hello

Hello John Smith!
```

## Events

Each message generated by this endpoint will trigger the
[http_message_generated](../events/http_message_generated.md) event.
