# preserve_intermediates

{{since('2025.03.19-1d3f1f67')}}

Boolean. When `true`, the resolver retains intermediate records (such as
CNAME entries chased during a lookup) in its response, rather than
returning only the terminal answer. Defaults to `true`.
