# validate

{{since('2025.03.19-1d3f1f67')}}

Boolean. Enables DNSSEC validation.

For the Hickory backend, queries are validated against the bundled DNSSEC
trust anchors.

For the Unbound backend, this loads the built-in DNSSEC trust anchors into
the unbound context. See also [trust_anchor_file](trust_anchor_file.md).

DANE for outbound SMTP requires DNSSEC validation; see
[enable_dane](../../kumo/make_egress_path/enable_dane.md).
