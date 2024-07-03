# How do I create an always-suspended queue?

Some sites employ a catch-all queue for messages that didn't get categorized
and queued into an expected queue. In the Momentum (AKA Ecelerity) MTA they might
do this by defining a binding or binding group that has delivery permanently
suspended.

The equivalent concept in KumoMTA is to define an Egress Pool that has no
sources, and associate the always-suspended queue with that pool.

That will cause messages that are queued to it to experience a transient failure
with a disposition of this form:

```
451 4.4.4 no non-zero-weighted sources available for QUEUE_NAME
```

Messages will be subject to the usual retry schedule, which you can of course
configure for your queue.

To configure this using the queues and sources helpers:

* Add the following to your `queues.toml`:

{% call toml_data() %}
[queue."always-suspended"]
egress_pool = "always-suspended-pool"
{% endcall %}

* Add the following to your `sources.toml`:

{% call toml_data() %}
[pool."always-suspended-pool"]
{% endcall %}

* Then in your policy, you can put messages in that queue explicitly:

```lua
kumo.on('smtp_server_message_received', function(msg)
  if some_condition then
    -- Explicitly assign the message to the always-suspended queue.
    -- Its envelope, tenant and campaign will be ignored as the queue
    -- meta value takes precedence
    msg:set_meta('queue', 'always-suspended')
  end
end)
```
