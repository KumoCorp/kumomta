# case_randomization

{{since('2025.05.06-b29689af')}}

Boolean. When `true`, the resolver randomizes the case of letters in query
names and requires that responses echo the same case. This implements the
mechanism described in
[draft-vixie-dnsext-dns0x20-00](https://datatracker.ietf.org/doc/html/draft-vixie-dnsext-dns0x20-00)
and is a mitigation against blind cache-poisoning attacks. Only applies
over UDP. Defaults to `false`.
