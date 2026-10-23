# MTA-STS

MTA-STS (Mail Transfer Agent Strict Transport Security, RFC 8461) is a policy mechanism that lets a receiving domain declare, via DNS and HTTPS, that senders must deliver to it over authenticated TLS. It defends against downgrade and man-in-the-middle attacks on server-to-server mail without requiring DNSSEC.
