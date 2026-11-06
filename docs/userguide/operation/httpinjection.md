---
description: Inject messages into KumoMTA over HTTP using a configured HTTP listener, controlling the IPs and ports available for message submission via the API.
---

# Injecting Using HTTP

KumoMTA will listen for message injection in any
[listener](../..//reference/kumo/start_esmtp_listener/index.md)
[defined](../../reference/kumo/start_http_listener/index.md) in
configuration. You have complete control over the IPs and Ports available for
message injection.

The HTTP Listener will accept any properly formatted HTTP connection request
allowed by its configuration.  For instance, based on this:

```lua
kumo.start_http_listener {
  use_tls = true,
  listen = '0.0.0.0:8005',
  -- allowed to access any http endpoint without additional auth
  trusted_hosts = { '127.0.0.1', '::1' },
}
```

KumoMTA will accept any HTTPS injection on port 8005 from the local host ONLY.
(This also enables the full [HTTP API](../../reference/http/kumod/index.md) from
localhost).


The simplest test of [HTTP injection](../../reference/http/kumod/api_inject_v1_post.md)
can be done using curl right from the localhost console. Note the `-k`
option: it lets curl accept the self-signed certificate that KumoMTA
generates when no TLS certificate is configured for the listener.

```console
$ curl -ik 'https://localhost:8005/api/inject/v1' \
 -H 'Content-Type: application/json' -d '
{"envelope_sender": "noreply@example.com",
 "content": "Subject: hello\n\nHello there",
 "recipients": [{"email": "recipient@example.com"}]
}'
```

That should return something like this:

```json
{"success_count":1,"fail_count":0,"failed_recipients":[],"errors":[]}
```

Any system that can use an HTTP API to pass JSON should work as an injection
system if you follow the JSON payload formatting rules posted
[here](../..//reference/http/kumod/api_inject_v1_post.md)

## Injecting Independent Messages in a Batch

Use `POST /api/inject/batch/v1` when you have several independently rendered
messages, each with its own sender, content, and recipients. It accepts an
ordered array of normal `/api/inject/v1` request bodies:

```json
{
  "requests": [
    {
      "envelope_sender": "sender@example.com",
      "content": "Subject: First\n\nFirst body",
      "template_dialect": "Static",
      "recipients": [{"email": "one@example.com"}]
    },
    {
      "envelope_sender": "sender@example.com",
      "content": "Subject: Second\n\nSecond body",
      "template_dialect": "Static",
      "recipients": [{"email": "two@example.com"}]
    }
  ]
}
```

Each batch entry also accepts the `template_dialect` and `substitutions`
fields supported by `/api/inject/v1`; the examples above use `Static` for
already-rendered content.

The entire batch is rejected with HTTP 422 if any sender or recipient
address is malformed, before any entry is processed. Template rendering and
queueing can still fail after this check.

A successful batch envelope returns HTTP 200 with a `results` array. Each
entry includes its original `index`, `status`, and either the original
`InjectV1Response` in `response` or an `error`. The batch is **not
atomic**: successful injections are not rolled back when
an entry fails.
Check each entry's `fail_count` and `failed_recipients` even if its
`status` is 200. If an entry returns HTTP 503, later entries are marked
as not attempted.

The endpoint accepts up to 100 injection requests and 10,000 recipients per
batch, subject to the HTTP listener's configured body-size limit. Requests
are processed sequentially using the existing injection handler, including
recipient throttling. A lost HTTP response gives no guarantee about which
injections were accepted; avoid blindly replaying the whole batch.
