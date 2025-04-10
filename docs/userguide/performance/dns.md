# DNS Performance

DNS is at the core of KumoMTA's processing pathway, it's used to validate incoming messages and their destination and to define queueing, and it's needed to successfully route messages to their MXes. 

DNS performance is critical, and if DNS performance is missing it can slow down the entire message flow.

## Use a Local Cacheing DNS Resolver

Due to the large volume of queries issued by KumoMTA it is strongly recommended that you use a local cacheing DNS resolver. Relying on external DNS providers introduces excess latency and potential service outages.

On most Linux distributions the default resolver is [Bind](https://en.wikipedia.org/wiki/BIND). Not all distributions maintain a current version of Bind, updating to the latest stable release is strongly recommended.

Tuning Bind for performance is beyond the scope of this document but is recommended.
