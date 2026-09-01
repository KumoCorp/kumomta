# ARC (Authenticated Received Chain)

ARC (RFC 8617) preserves authentication results across intermediaries that legitimately modify mail, such as mailing lists and forwarders, by having each hop sign the state it received. It lets a final receiver trust that a message which now fails SPF/DKIM was authentic before the intermediary touched it.
