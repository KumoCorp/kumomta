# Egress proxy (for sending)

An egress proxy terminates outbound SMTP connections on behalf of MTA nodes so that mail exits from stable, reputation-bearing IPs regardless of where the sending compute runs. Proxying (via HAProxy or SOCKS, for example) is the standard pattern for running MTAs in Kubernetes or autoscaling environments where pod IPs are ephemeral.
