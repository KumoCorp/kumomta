# num_concurrent_reqs

{{since('2025.03.19-1d3f1f67')}}

Integer. When more than one nameserver is configured, how many of them to
query in parallel for a single lookup. `0` or `1` means queries are issued
serially. Defaults to `2`.
