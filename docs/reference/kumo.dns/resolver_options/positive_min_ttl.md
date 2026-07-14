# positive_min_ttl

{{since('2025.03.19-1d3f1f67')}}

Duration string. Lower bound for positive-response TTLs. Responses with a
TTL below this value are cached as if their TTL were `positive_min_ttl`.
Useful for clamping aggressive zero-TTL upstreams. Unset by default.
