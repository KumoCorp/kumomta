# DKIM selector

A DKIM selector is the label that identifies which of a domain's published signing keys a signature uses; the receiver fetches the public key from selector._domainkey.domain in DNS. Selectors let a domain hold many keys at once, one per mail stream, vendor, or key generation, which is what makes routine key rotation and per-provider key isolation workable.
