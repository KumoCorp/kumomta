---
description: "Reclassify a bounce in KumoMTA — demote a 5xx to transient with classifier rules or smtp_client_rewrite_delivery_status, and how rule precedence works."
---

# How Do I Reclassify a Bounce (Make a 5xx Transient Instead of Permanent)?

Sometimes a mailbox provider returns a `5xx` that you would rather treat as temporary — a reputation or complaint block you expect to clear, where you want the message retried instead of hard-bounced. There are two ways to do it.

## Option 1: classifier rules

Override the classification by adding a rule, loaded before the defaults, that maps the response to a transient class. The classifier matches against the normalized full response (status code plus content):

```toml
# custom_bounces.toml — loaded before iana.toml
[rules]
TransientFailure = [
  "blacklisted",
  "PolicyRelated",
]
```

```lua
kumo.configure_bounce_classifier {
  files = {
    '/opt/kumomta/etc/custom_bounces.toml', -- your overrides first
    '/opt/kumomta/share/bounce_classifier/iana.toml',
  },
}
```

## Option 2: rewrite the response

Demote the SMTP response directly in policy, which keeps the message in retry:

```lua
kumo.on('smtp_client_rewrite_delivery_status', function(response, domain, ...)
  if response:find '550' and response:find 'blacklisted' then
    return 454 -- treat as transient
  end
end)
```

## Rule precedence

If more than one rule matches, the rule from the file loaded first wins, so define your overrides before `iana.toml`. Prefer matching the meaningful phrase from the community `bounces.toml` rather than building everything from scratch.

!!! note
    Connection-time failures (errors that occur before any per-message data is sent) are always treated as transient transport errors. KumoMTA cannot attribute them to a specific recipient.

There is no built-in validator for the classifier file. Lint the TOML, load it on one node, and watch for an acceptable rate of unclassified bounces before rolling it out.

## See also

* [Configuring Bounce Classification](../userguide/configuration/bounce.md)
* [smtp_client_rewrite_delivery_status](../reference/events/smtp_client_rewrite_delivery_status.md)
