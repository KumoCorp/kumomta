# MX (Mail Exchanger) record

An MX record is the DNS record that declares which servers accept email for a domain, in priority order. When an MTA delivers a message, it looks up the recipient domain's MX records and connects to the listed hosts. A domain's MX is its inbound front door; MX records say nothing about who sends mail for the domain.
