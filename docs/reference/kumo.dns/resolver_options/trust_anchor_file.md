# trust_anchor_file

{{since('2025.05.06-b29689af')}}

String. Path to a DNSSEC trust anchor file containing one or more `DS` or
`DNSKEY` records. Loaded into the resolver in addition to (or instead of)
the built-in root trust anchors when [validate](validate.md) is enabled.

For most deployments, leaving this unset and setting `validate = true` (which
uses the bundled root trust anchors) is sufficient.

{{since('dev', inline=True)}}: the corresponding hickory option was named
`trust_anchor`; KumoMTA renames it to `trust_anchor_file` to make the file-path
nature explicit.
