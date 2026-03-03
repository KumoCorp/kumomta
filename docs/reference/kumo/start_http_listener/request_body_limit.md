# request_body_limit

{{since('2024.06.10-84e84b89')}}

Specifies the maximum acceptable size of an incoming HTTP request, *after*
decompressing any compressed body, in bytes.  This option limits the RAM
usage rather than the wire usage.

The default limit is 2MB.

If an incoming request exceeds this limit, a `413 Payload Too Large` HTTP
response will be returned, with the body `Failed to buffer the request body:
length limit exceeded`.

