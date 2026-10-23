# STARTTLS

STARTTLS is the SMTP extension that upgrades a plaintext connection to an encrypted TLS session. It is opportunistic by default, meaning mail still flows if TLS fails, which is why policies like MTA-STS and DANE exist to make encryption enforceable.
