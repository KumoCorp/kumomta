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

## Compressed requests

The HTTP listener accepts request bodies compressed with gzip, deflate (zlib),
or Zstandard ({{since('dev', inline=True)}}). Compress the JSON payload and set
`Content-Encoding` to `gzip`, `deflate`, or `zstd`, respectively, while keeping
`Content-Type: application/json`. Zstandard bodies must not depend on an external
compression dictionary.
For example, to submit an existing `payload.json` using Zstandard:

```console
$ zstd -1 -c payload.json | curl --fail-with-body \
  'https://mta.example.com:8005/api/inject/v1' \
  -H 'Content-Type: application/json' \
  -H 'Content-Encoding: zstd' \
  --data-binary @-
```

Use the authentication required by your listener configuration. Uncompressed
requests remain supported. Request decompression is shared by the HTTP listeners
used by KumoMTA, TSA, and the proxy.

The listener's
[`request_body_limit`](../../reference/kumo/start_http_listener/request_body_limit.md)
applies to the **decompressed JSON body**, not its compressed size on the wire.
The default is 2 MiB; requests exceeding the limit receive `413 Payload Too Large`.
This limit does not bound all decoder memory (such as the Zstandard frame's
window) or decompression CPU usage. Decoding also occurs before the injection
concurrency limit is acquired, so size listener admission controls and client
concurrency accordingly.

## Compressed responses

Response compression is negotiated independently from request compression.
Send `Accept-Encoding: zstd` to allow Zstandard responses
({{since('dev', inline=True)}}), or advertise gzip and deflate according to your
client's capabilities. Eligible responses are compressed using a supported
encoding and returned with the corresponding `Content-Encoding` header. Your
client must decode that encoding before processing the response.

A compressed request does not imply a compressed response. Clients that do not
advertise Zstandard, or explicitly set `zstd;q=0`, will not receive Zstandard
responses. Small responses, including some injection results, may remain
uncompressed even when compression is advertised. See the
[HTTP listener](../../reference/kumo/start_http_listener/index.md) documentation
for details shared by all HTTP endpoints.
