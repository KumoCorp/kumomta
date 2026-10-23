# edns0

{{since('2025.03.19-1d3f1f67')}}

Boolean. Enables EDNS0, which permits larger UDP responses (typically up to
the negotiated EDNS payload size rather than the legacy 512 byte limit).
Required in practice for DNSSEC and for DKIM/MTA-STS/DMARC TXT lookups
that may be larger than 512 bytes. Defaults to `true`.
