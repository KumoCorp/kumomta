# negative_min_ttl

{{since('2025.03.19-1d3f1f67')}}

Duration string. Lower bound for negative-response (NXDOMAIN, NODATA) TTLs.
Responses with a TTL below this value are cached as if their TTL were
`negative_min_ttl`. Unset by default.
