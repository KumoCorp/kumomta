# DMARC

DMARC (RFC 7489) is a policy layer on top of SPF and DKIM: it lets a domain owner declare what receivers should do with mail that uses the domain in the visible From header but fails authentication (none, quarantine, or reject), and requests aggregate reports on who is sending as the domain. DMARC is what makes From-domain spoofing enforceable, and major mailbox providers now require bulk senders to publish a DMARC policy, with p=none as the accepted minimum.
