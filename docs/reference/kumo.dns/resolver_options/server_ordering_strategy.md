# server_ordering_strategy

{{since('2025.03.19-1d3f1f67')}}

String. How the resolver chooses which configured name server to try first
for each query. One of:

* `QueryStatistics` — order by recent success/latency statistics.
* `RoundRobin` — round-robin across the configured list.
* `UserProvidedOrder` — always try servers in the order they were
  configured.

Defaults to `QueryStatistics`.
