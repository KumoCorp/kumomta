# max_mx_addresses_per_host

{{since('dev')}}

The largest number of addresses to retain from one MX host.

When resolving a destination, each MX host is expanded to its `A`/`AAAA`
addresses. This caps how many of those addresses are kept for one host.

This cap is applied before the overall
[max_mx_plan_size](max_mx_plan_size.md) cap.

The default value is `10`.
