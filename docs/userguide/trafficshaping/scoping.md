# Scoping Traffic Shaping Rules

When KumoMTA needs to connect to a remote host to deliver messages, the [get_egress_path_config](../../reference/events/get_egress_path_config.md) event is fired in order to determine what configuration to use for that specific pathway.

```lua
kumo.on(
  'get_egress_path_config',
  function(routing_domain, egress_source, site_name)
    return kumo.make_egress_path {
      enable_tls = 'OpportunisticInsecure',
    }
  end
)
```

The event passes three attributes that are keyed to determine the desired traffic shaping rules, namely the `routing_domain`, `egress_source`, and `site_name`.

Where the `routing_domain` in the event call is the actual domain from the recipient address (for example, corp.com for a message destined to `user@corp.com`), the site_name is defined in the server by building an expression that represents all MX servers returned in an MX lookup for the `domain`. For example, if `user@corp.com` is hosted by Google Workspace then the `site_name` might be something similar to `(alt1|alt2|alt3|alt4)?.aspmx.l.google.com`.

The site_name concept allows managing traffic more effectively for MBPs that host a large number of domains. Rather than treating each domain as a separate destination with its own queues and traffic shaping counters, the traffic can be grouped and shaped as a single MX group, which is what the MBPs expect when they receive incoming traffic.

For example: if a sender wanted to limit connections to 10 per domain, and Google Workspace hosted 1,000 domains in the sender's queues, a server without MX rollup or sitenames would open 10,000 connections (1000 domains * 10 connections each). With the use of `site_name`, KumoMTA merges the 1,000 domains under a single `site_name` and maintains the limit of 10 connections per `egress_source` to the Google Workspace servers.

Messages in the Ready Queue are grouped into separate queues based on the combination of `egress_source` and `site_name`. The `routing_domain` is provided for convenience when working out what parameters to use.

!!! note
    It is important to understand that while KumoMTA will build queues based on a `site_name`, it is not expected that the end user will configure traffic shaping using a `site_name`. Instead, configuration is done using a domain identifier that belongs to a given `site_name`, and the generated `site_name` is compared to it, as is done in the `shaping.lua` helper.

    For example, when using the helper to configure traffic shaping for the Yahoo! domains, a user would configure traffic shaping for `yahoo.com`, knowing that all domains that have a matching `site_name` would also have the same traffic shaping configuration applied.
