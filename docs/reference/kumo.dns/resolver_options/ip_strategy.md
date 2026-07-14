# ip_strategy

{{since('2025.03.19-1d3f1f67')}}

!!! caution
    This option is almost certainly **not** what you want. It controls a
    hickory-internal `lookup_ip` API that KumoMTA does not use during normal
    MX-host resolution.

    To control IP address family selection when resolving MX hosts, use
    [ip_lookup_strategy](../../kumo/make_egress_path/ip_lookup_strategy.md)
    on the egress path instead. Setting it in the egress path also lets you
    vary the policy per source.

Controls how the hickory resolver queries for A and/or AAAA records
when its own dual-stack `lookup_ip` entry point is invoked. One of:

* `Ipv4Only` — only query A records.
* `Ipv6Only` — only query AAAA records.
* `Ipv4AndIpv6` — query both in parallel, returning A entries before AAAA.
* `Ipv6AndIpv4` — query both in parallel, returning AAAA entries before A.
* `Ipv4thenIpv6` — query A first; only query AAAA if A returned nothing.
* `Ipv6thenIpv4` — query AAAA first; only query A if AAAA returned nothing.

Defaults to `Ipv6AndIpv4`.
