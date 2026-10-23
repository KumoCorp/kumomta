# Horizontal scaling

Horizontal scaling is adding nodes rather than bigger nodes. For outbound MTAs the practical questions are how sending IPs, queues, and rate limits are coordinated across nodes, and whether shaping decisions are shared. Per-node limits that ignore cluster-wide totals will exceed provider tolerances.
