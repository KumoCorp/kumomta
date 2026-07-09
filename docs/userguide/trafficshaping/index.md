# Traffic Shaping

<iframe width="560" height="315" src="https://www.youtube.com/embed/Vxbe5ExMOXk?si=2SC7o8FObyvWqavl" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

By default, the KumoMTA server will deliver messages as quickly as possible to each destination, with few restrictions regarding number of connections, number of messages per connection, or number of messages per second. Because unthrottled sending is unwelcome to most MailBox Providers (MBPs) it is highly recommended that KumoMTA users implement traffic shaping in order to limit sending speeds to something more aligned with the expectations of the individual MBPs.

Common throttles include concurrent connection limits, messages per connection, rate of opening connections, and rate of sending messages. In addition, users can set options for various timeouts, queue sizes, and what encryption rules to use when communicating with remote hosts.

## Configuring An Egress Path

All messages are relayed to external hosts along an **egress path**, which is defined as a combination of a **routing domain**, **egress source**, and a **site name**, triggered via the `get_egress_path_config` event.

In response to the `get_egress_path_config` event, the user calls `kumo.make_egress_path` and provides the desired parameters for the specific combination of source and destination:

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    max_connection_rate = '100/min',
  }
end)
```

The full list of parameters that can be configured for a given egress path can be found in the [make_egress_path](../../reference/kumo/make_egress_path/index.md) page of the [Reference Manual](../../reference/index.md).

## The `shaping.lua` Helper

While users are free to implement their traffic shaping rules as they see fit, the KumoMTA team has provided the `shaping.lua` helper as a pre-built implementation based on static configuration files in either JSON or TOML format, with support for various configuration scopes and automated rules using Traffic Shaping Automation.

The remainder of this chapter is focused on the use of the `shaping.lua` helper.

## Traffic Shaping Automation

Many of the largest MBPs operate platforms that provide feedback to senders through their response codes during the SMTP conversation. This feedback will include information related to the traffic shaping patterns in use by the sender, including bounces for too many connections, too many messages per connection, sending rate, and sender reputation.

To ensure optimum throughput and deliverability, KumoMTA features Traffic Shaping Automation (TSA) that monitors responses from remote hosts and adjusts traffic shaping rules on a granular level in real time.

## In This Chapter

* [Scoping Traffic Shaping Rules](./scoping.md) — How KumoMTA uses domain, egress source, and site name to scope traffic shaping rules.
* [MX Rollups and Provider Blocks](./rollups.md) — How domain entries map to site names, and using provider blocks for pattern-based MX matching.
* [Traffic Shaping Configuration Files](./shapingfiles.md) — Configuring shaping rules using the `shaping.lua` helper and TOML/JSON configuration files.
* [Shaping Option Resolution Order and Precedence](./resolution.md) — How shaping options are merged and resolved across multiple configuration layers.
* [Writing Custom Shaping Files](./customshaping.md) — Creating your own traffic shaping rules to supplement or replace the defaults.
* [Traffic Shaping Automation](./automation.md) — Deploying the TSA daemon for automated, real-time traffic shaping adjustments.
* [Testing Your Shaping Files](./testing.md) — Validating shaping file syntax using the built-in `validate-shaping` tool.
