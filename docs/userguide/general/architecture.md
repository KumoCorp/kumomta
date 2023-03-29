KumoMTA was built with large-scale sending in mind.  Here are a few key ideas
that shape the implementation:

* Designed for modern multi-core, multi-threaded systems. Built on top of
  [Tokio](https://tokio.rs/), the core IO scheduler will use all available
  parallelism.
* High Performance spool. We recommend using the [RocksDB](https://rocksdb.org/)
  based spool for a combination of in-memory buffering, write-ahead logging and
  asynchronous data flushing that enable the best performance while minimizing
  the risk of low-durability deferred spooling solutions.
* KumoMTA has first-class support for queuing based on the combination of
  *tenant*, *campaign* and destination site.  Having separate queues make it
  easier to see and manage your traffic.
* *Destination Site* concept makes it easier to shape traffic to big receiving sites
  that provide service for many domains. Rather than shaping based on just the
  domain name, KumoMTA will traverse the MX records for the destination and use
  that information as the basis for shaping. As a result, KumoMTA sees all
  G-suite hosted domains as going to the same destination without requiring
  any static configuration.  The same approach works for any domains that
  share identical MX records, not just G-suite.
* Powerful Configuration. No limiting, bespoke, domain-specific configuration files
  here! KumoMTA embeds the Lua language to express both declarative configuration
  as well as enable you to express more advanced configuration to match your
  policy or setup.
* Composable and easy-to-reason-about extensibility. There are very few implicit
  behaviors or actions, and those that exist are easy to control or disable.
  This design principle means that new features can be delivered as new
  functions or new modules that you can trigger from your policy configuration
  if you wish to use them.

