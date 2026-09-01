# DKIM (DomainKeys Identified Mail)

DKIM (RFC 6376) is a cryptographic signature standard: the sending infrastructure signs each message with a private key, and receivers verify the signature against a public key published in the signing domain's DNS. A valid DKIM signature proves the message is authorized by the signing domain and unmodified in transit. Signing at scale, with key management, rotation, and per-tenant signing domains, is a core MTA capability.
