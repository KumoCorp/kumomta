# SPF (Sender Policy Framework)

SPF (RFC 7208) is a DNS-based standard that lists which servers are authorized to send mail using a domain in the envelope sender. Receivers check the connecting IP against the domain's published SPF record. An SPF check is limited to 10 DNS lookups, a budget that include: mechanisms consume and that stacked vendor records routinely exhaust. SPF alone breaks on forwarding and says nothing about the visible From header, which is why it is combined with DKIM under DMARC.
