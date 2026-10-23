# timeout

{{since('2025.03.19-1d3f1f67')}}

Duration string (for example `"5s"`, `"500ms"`). Per-query timeout for an
individual request to an upstream nameserver. Does not bound the total time
across retries; see [attempts](attempts.md). Defaults to `5s`.
