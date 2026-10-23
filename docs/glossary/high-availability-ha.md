# High availability (HA)

High availability is designing sending infrastructure to survive component failure without losing accepted mail or stopping the flow: redundant MTA nodes, durable spools, and failover for injection endpoints. Because SMTP has retry built in, email HA is more forgiving than request/response systems, but accepted-but-unspooled mail is unrecoverable and thus the thing HA design must make impossible.
