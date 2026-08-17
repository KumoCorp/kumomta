---
description: Understand the order and precedence KumoMTA uses to resolve traffic shaping options, merging default, provider, site name, domain, and source blocks.
---

# Shaping Option Resolution Order and Precedence

When resolving the configuration for a site, the options are resolved in the
following order:

1. The values for the `default` domain block are taken as the base
2. Any matching `provider` blocks are then merged in
3. Any matching `provider` + `source` blocks for the current source are merged in
4. Any matching _site name_ blocks are merged in. These are domain blocks that have the default (implied) or explicitly configured `mx_rollup = true` option set in them.
5. Any matching domain blocks are merged in. These are domain blocks that have `mx_rollup=false` set in them.
6. Any matching _site name_ + `source` blocks are merged.
7. Any matching domain + `source` blocks are merged.

Within any of these steps above, the options are merged in the order that they
appear across your configuration files, so the most recently specified value
will take precedence overall.

You can specify `replace_base=true` in a block to have that block override the
current set of accumulated values.

!!! warning
    There is currently no mechanism for unsetting an option previously merged in. If there is a throttle set earlier (for example in `[default]`) that you wish to unset rather than explicitly define a different throttle then you **must** use `replace_base=true` to replace all previously merged options.

Most options merge directly over the top of earlier options, but the
[additional_connection_limits](../../reference/kumo/make_egress_path/additional_connection_limits.md) and
[additional_message_rate_throttles](../../reference/kumo/make_egress_path/additional_message_rate_throttles.md)
options merge the maps together.
