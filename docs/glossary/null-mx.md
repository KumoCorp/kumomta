# Null MX

A null MX record (RFC 7505) declares in DNS that a domain never accepts email, using a single MX record with priority zero and a "." target. Publishing it lets sending servers fail immediately instead of retrying for days against a domain with no mail service, and it keeps non-mail domains from being useful in forged bounce addresses.
