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
            "email": recipient@example.com",
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

The following fields are defined for the inject request:

## content

Specifies the message content. It can either be a string value or
a JSON object describing how to build a the message.

If a simple string is provided, it must be an RFC822 compliant
message.  If template substitutions are used in the request, then
the entire RFC822 message string is used as-is for the template;
no message parsing or decoding is performed as part of template
expansion.

Alternatively the content can be specified as a JSON object as
demonstrated below.

```admonish
Comments are used inline in the JSON objects on this page for the purposes of
exposition in these docs, but comments are not valid in the actual request.
```

```json
{
    "envelope_sender": "noreply@example.com",
    "content": {
        "text_body": "This is the plain text part",
        "html_body": "<p>This is the <b>HTML</b> part</p>",
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
                "file_name": "pixel.gif",
            }
        ],
        "headers": [
            // You can use separate name/value...
            ["Subject", "This is the subject"],
            // ... or a string with the entire header name and value
            "From: \"Someone\" <someone@example.com>"
        ]
    },
    "recipients": [
        {
            "email": "recipient@example.com",
        }
    ]
}
```

When building a message, template substitutions are applied to the *text_body*,
*html_body* and *headers* fields.  Attachments are not subject to template
substitution.

## envelope_sender

The address to use as the envelope sender address when generating
the message.

It must be a string of the form *user@domain*.

## recipients

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
            "name": "Mr. Recipient"
            // Optional additional template substitutions
            "substitutions": {
                "key": "value",
            }
        }
    ]
}
```

## substitutions

Specifies a set of global substitutions to for template expansion:

```json
{
    "substitutions": {
        "campaign": "Summer Sale 2023",
    }
}
```

# Template Substitution

The injection API embeds the [Mini
Jinja](https://docs.rs/minijinja/latest/minijinja/) templating engine.  The
full supported syntax is [documented
here](https://docs.rs/minijinja/latest/minijinja/syntax/index.html).

For each recipient, the set of variables pre-defined in the template are:

* The set of global substitutions defined by `request.substitutions`

* The set of per-recipient substitutions, if any are defined in
  `request.recipients[].substitutions`, are overlaid and take precedence over
  any global substitutions

* The recipient `name` and `email` fields are assigned to the `"name"` and
  `"email"` variables respectively.

```admonish
Both sets of *substitutions* can use any JSON value for the values of
the variables; they don't have to be strings.
```

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

