# Traffic shaping

Traffic shaping is the destination-aware control of outbound sending behavior: how many connections to open to a given provider, how many messages per connection, how fast, from which IPs, with what retry and backoff behavior. It exists because every mailbox provider tolerates different behavior and punishes senders who exceed it. Fine-grained shaping, per provider, per IP pool, and per tenant, is the defining capability that separates high-volume outbound MTAs from general-purpose mail servers.
