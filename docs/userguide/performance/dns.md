---
description: Optimize DNS performance for KumoMTA with a local caching resolver, since DNS lookups are central to validating, queueing, and routing every message.
---

# DNS Performance

DNS is at the core of KumoMTA's processing pathway, it's used to validate incoming messages and their destination and to define queueing, and it's needed to successfully route messages to their MXes.

DNS performance is critical, and if DNS performance is poor it can slow down the entire message flow.

## Use a Local Caching DNS Resolver

Due to the large volume of queries issued by KumoMTA it is strongly recommended that you use a local caching DNS resolver. Relying on external DNS providers introduces excess latency and potential service outages.

On most Linux distributions the default resolver is [BIND](https://en.wikipedia.org/wiki/BIND). Not all distributions maintain a current version of BIND; updating to the latest stable release is strongly recommended.

Tuning BIND for performance is recommended, but is beyond the scope of this document.
