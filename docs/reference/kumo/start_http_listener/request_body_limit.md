# request_body_limit

{{since('2024.06.10-84e84b89')}}

Specifies the maximum acceptable size of an incoming HTTP request, *after*
decompressing any compressed body, in bytes.  This option limits the RAM
used to buffer the decoded body rather than the wire usage. It does not cap all
decoder memory, such as a Zstandard frame's window, or decompression CPU usage.

The default limit is 2MB.

If an incoming request exceeds this limit, a `413 Payload Too Large` HTTP
response will be returned, with the body `Failed to buffer the request body:
length limit exceeded`.
