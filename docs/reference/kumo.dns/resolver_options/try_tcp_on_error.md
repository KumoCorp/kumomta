# try_tcp_on_error

{{since('2025.03.19-1d3f1f67')}}

Boolean. When `true`, if a query against a name server's UDP connection
errors, the resolver immediately retries the same query over that name
server's TCP connection (if one is configured) before consulting other
name servers. Defaults to `false`.
