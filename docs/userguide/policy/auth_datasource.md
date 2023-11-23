# Validating SMTP AUTH Requests Against a Data Source

When hosting relay users it is important to protect your infrastructure from malicious senders, often without the ability to whitelist the IP addresses of legitimate users.

A common use case for relay hosts is validating AUTH credentials against a datasource for more dynamic management of sending users.

https://docs.kumomta.com/reference/kumo/memoize/?h=memoize