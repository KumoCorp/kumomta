# Queue architecture

Queue architecture is how an MTA organizes queued mail: classically one shared queue, or in modern outbound MTAs, separate logical queues per destination provider, tenant, campaign, or IP pool. Granular queues prevent one blocked destination or one customer's bad campaign from delaying everyone else's mail, and allow policy (retry, throttle, expiry) to be set per queue.
