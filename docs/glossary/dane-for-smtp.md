# DANE (for SMTP)

DANE (DNS-Based Authentication of Named Entities, RFC 7672 for SMTP) publishes TLS certificate bindings in DNSSEC-signed DNS (TLSA records), letting a sending MTA verify it is speaking to the genuine receiving server and enforce encryption. It is the DNSSEC-based alternative and complement to MTA-STS.
