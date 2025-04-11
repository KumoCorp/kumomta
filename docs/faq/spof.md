# How Do I Avoid Having a Single Point of Failure?

In high-volume sending environment uptime becomes critical, and one key to that is to avoid having a Single Point of Failure (SPOF).

The following can help keep your email infrastructure highly available:

* Place your KumoMTA instances behind load balancers. A common architectural choice is to configure message generators that are assigned to a single specific MTA node, or even to assign individual tenants to a single MTA node. KumoMTA is cluster-aware and able to share tenants and throttles across all nodes. 
* If you use a public cloud provider, leverage their hosted implementations of services such as Redis and AMQP. These implementations have extensive fault tolerance built into them that can help ensure continuous uptime.
* Ensure that your public IPs are highly available. If you use KumoProxy or HAProxy you will need to ensure that the public IPs attached to them are highly available. For an example on how to implement IP failover for proxy nodes see [this article](https://www.digitalocean.com/community/tutorials/how-to-set-up-highly-available-web-servers-with-keepalived-and-reserved-ips-on-ubuntu-14-04) (note that the article is written with Nginx as the monitored service but it can also apply to proxy servers). Another alternative is to leverage the RNAT or port mapping capabilities of your enterprise firewall, as those systems generally have very high availability.
* Load-balance your webhook consumers. One common SPOF is having a single server consuming webhooks. This can not only lead to overload during busy times, it creates a potential SPOF if the consumer goes down. While KumoMTA will retry delivery of webhooks, an extended outage can result in a full spool partition.