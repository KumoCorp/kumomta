# Spool

The spool is the MTA's durable on-disk store for queued messages: mail that has been accepted but not yet delivered. Spool performance (write throughput, recovery after restart) bounds an MTA's safe injection rate, and spool durability is what makes "accepted" a meaningful promise.
