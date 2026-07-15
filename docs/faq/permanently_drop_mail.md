---
description: "Stop or drop queued mail with kcli bounce — why admin bounces are transient, how to flush immediately, and how to make a rule permanent."
---

# How Do I Permanently Stop or Drop Queued Mail?

The admin bounce (`kcli bounce`) is the tool for dropping queued mail in an emergency, and it is transient: it applies only for the duration you specify and is forgotten after a restart. It is an operator "stop the bleeding" command, not a permanent policy.

## Drop mail now

```console
# Drop everything (irreversible)
$ kcli bounce --everything --reason 'purging all queues'

# Drop a specific destination domain
$ kcli bounce --domain yahoo.com --reason 'bad send'

# Drop a campaign
$ kcli bounce --campaign 'Back to school' --reason 'bad send'
```

A bounce stays active for its `--duration` (the default is 5 minutes), so it catches both currently-queued messages and any that arrive during that window. Set a duration at least as long as your retry interval if you want to keep catching retries.

## Make it take effect immediately

Bounced messages keep their existing next-attempt time unless you flush them. Follow the bounce with a rebind to apply it right away:

```console
$ kcli rebind --everything --always-flush
```

## Cancelling scheduled mail

There is no separate "unschedule" command. To cancel messages scheduled with `msg:set_scheduling()`, use an admin bounce scoped to the matching domain, campaign, or tenant. It operates on the Scheduled Queue as well.

!!! warning
    `kcli bounce` matches on the **recipient** domain/campaign/tenant. To drop mail by **sender** domain, match `msg:sender().domain` in a `rebind_message` handler and run `kcli rebind`.

## Making it permanent

Because the admin bounce evaporates on restart, encode any lasting "never send this" rule in your Lua policy — reject at reception, or reject inside a `rebind_message` handler — rather than relying on a standing bounce.

## See also

* [Cancelling Queued Messages](../userguide/operation/cancel.md)
* [How do I flush a queue?](flush_queue.md)
