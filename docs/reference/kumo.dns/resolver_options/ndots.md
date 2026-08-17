# ndots

{{since('2025.03.19-1d3f1f67')}}

Integer. The number of dots that must appear in a name (other than a trailing
dot representing the root) before the resolver treats it as fully qualified
and skips the `domain`/`search` list. Matches the `ndots` directive in
`/etc/resolv.conf`. Defaults to `1`.
