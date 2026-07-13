# ip_lookup_strategy

{{since('2026.04.09-ea3b2a9b')}}

Influences how MX host names are resolved to IP addresses.
This value is a string that can have one of the possible values listed below.

The default value is `Ipv4AndIpv6` which is how the product behaved
prior to the introduction of this option.

 * `Ipv4AndIpv6` - Both the `A` and `AAAA` records are resolved
   concurrently, with both being used to produce the effective list of
   addresses for that MX.  This is the default behavior.
 * `Ipv4Only` - only the `A` records will be resolved.
 * `Ipv6Only` - only the `AAAA` records will be resolved
 * `Ipv6ThenIpv4` - resolve the `AAAA` records first. If none are found
   or there is an error, resolve instead the `A` records.
 * `Ipv4ThenIpv6` - resolve the `A` records first. If none are found or
   there is an error, resolve instead the `AAAA` records.

If no `A` or `AAAA` records can be resolved for any of the MX hosts,
messages are considered to be non-deliverable for the associated egress
path and the corresponding ready queue will be bulk failed with a
`TransientFailure` with a `451 4.4.4 MX didn't resolve to any hosts`
response.

