# kumo.start_http_listener

```lua
kumo.start_http_listener { PARAMS }
```

Configure and start HTTP service.

This function should be called only from inside your
[init](../../events/init.md) event handler.

`PARAMS` is a lua table that can accept the keys listed below.

## HTTP compression

HTTP listeners support gzip and deflate (zlib) request decompression and response
compression. Zstandard support in both directions {{since('dev', inline=True)}}.
This behavior is shared by KumoMTA, TSA, and proxy HTTP listeners.

For **requests**, send a compressed body with `Content-Encoding: gzip`, `deflate`,
or `zstd`. Retain the content type of the decoded payload, for example
`Content-Type: application/json`. Zstandard bodies must be decodable without an
external compression dictionary. Unsupported content encodings receive
`415 Unsupported Media Type`; invalid compressed bodies are rejected when the
endpoint reads them.

For **responses**, advertise the encodings your client can decode using
`Accept-Encoding`. The listener selects a supported encoding according to the
advertised quality values, and returns it in `Content-Encoding`, along with
`Vary: Accept-Encoding` for compressed responses. `zstd;q=0` excludes Zstandard.
Clients that do not advertise Zstandard do not receive Zstandard responses.
Request and response encodings are independent.

The existing fast compression setting is retained. The default compression
predicate excludes responses shorter than 32 bytes, images other than SVG,
gRPC other than gRPC-Web, and server-sent events. Responses whose size is not
known in advance may still be compressed.

[`request_body_limit`](request_body_limit.md) bounds decoded request-body
buffering, not total decompression memory or CPU. Decoder window memory is
independent of this limit. Compression does not change authentication or
authorization requirements.

## HTTP Listener Parameters { data-search-exclude }
