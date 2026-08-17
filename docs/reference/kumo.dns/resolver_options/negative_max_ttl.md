# negative_max_ttl

{{since('2025.03.19-1d3f1f67')}}

Duration string. Upper bound for negative-response (NXDOMAIN, NODATA) TTLs.
Responses with a TTL above this value are cached as if their TTL were
`negative_max_ttl`. Unset by default.
