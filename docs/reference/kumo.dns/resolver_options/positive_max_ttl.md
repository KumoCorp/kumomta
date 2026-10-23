# positive_max_ttl

{{since('2025.03.19-1d3f1f67')}}

Duration string. Upper bound for positive-response TTLs. Responses with a
TTL above this value are cached as if their TTL were `positive_max_ttl`.
Useful for forcing periodic re-resolution of long-lived upstream records.
Unset by default.
