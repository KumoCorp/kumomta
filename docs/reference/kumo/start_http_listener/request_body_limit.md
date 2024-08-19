# request_body_limit

{{since('2024.06.10-84e84b89')}}

Specifies the maximum acceptable size of an incoming HTTP request, in bytes.
The default is 2MB.

If an incoming request exceeds this limit, a `413 Payload Too Large` HTTP
response will be returned.

