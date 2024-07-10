# Configuring Queue Rollup

By default, KumoMTA will perform automatic "rollup" of the ready queue based on
the *site name* that it derives from the MX records for the destination domain,
which means that your shaping rules will automatically apply across sites that
share the same MX hosts with no additional configuration required.

Some destination sites, notably `outlook.com`, use a DNS scheme that doesn't
work well with this automatic MX-based rollup scheme.

KumoMTA ships with a `policy-extras.rollup` module that can be used to
provide an alternative rollup scheme for these domains.

!!! warning
    Using any kind of rollup scheme other than the automatic MX-based
    site name approach exposes you to the potential for incorrectly
    routing mail. For example, some Microsoft domains have regional
    associations that result in policy rejections when you send to
    a host other than what would normally be returned from the
    associated MX records. There is no automatic way for KumoMTA
    to see through those sorts of site-specific logic from the outside.
    **It is your responsibility to employ these techniques only where
    they are valid and appropriate** and be prepared to adjust this
    configuration as the policies of the destination site change
    over time.

## Outlook and Hotmail MX records

Let's consider the MX records for `hotmail.com` and `outlook.com`, which are
both owned and operated by the same entity and infrastructure; at the time of
writing they look like this:

```console
$ dig +short mx hotmail.com
2 hotmail-com.olc.protection.outlook.com.
$ dig +short mx outlook.com
5 outlook-com.olc.protection.outlook.com.
```

and their site names:

```console
$ /opt/kumomta/sbin/resolve-site-name hotmail.com
hotmail-com.olc.protection.outlook.com
$ /opt/kumomta/sbin/resolve-site-name outlook.com
outlook-com.olc.protection.outlook.com
```

What this means is that, by default, mail sent to outlook and hotmail will
egress through separate ready queues (because the site names are different),
and be subject to separate shaping rules and separate limits/throttles, which
may increase the chances of exceeding the sending rate desired by that
destination site.

## MX host suffix rollup

This technique analyzes the host names returned from the MX record
of the destination domain and compares them against a mapping table
of *hostname suffix* to *routing domain*.  If every hostname in
the set of MX records matches a suffix in the mapping table, then
it is considered to be an overall match and the `routing_domain`
meta value is set to the corresponding domain.

```lua
local rollup = require 'policy-extras.rollup'

kumo.on('smtp_server_message_received', function(msg)
  rollup.reroute_using_ip_rollup(msg, {
    ['.olc.protection.outlook.com.'] = 'outlook.com',
  })
end)
```

With this configuration in place mail sent to outlook and hotmail will now
egress through the same ready queues; hotmail will be treated as though it was
`outlook.com` because its `routing_domain` will be set to `outlook.com`.  This
configuration will match ANY destination domains whose MX hosts all end with
`.olc.protection.outlook.com`, not just hotmail and outlook.  Your
`outlook.com` shaping rules would then apply across all domains that are
matched and rerouted by this module.

You will see the routing manifest in the queue names when you look at the metrics
and/or `kcli queue-summary` output; instead of the scheduled queue name being
`hotmail.com` it will appear as `hotmail.com!outlook.com` to indicate that it
will be routed via `outlook.com`.

You can define multiple entries in your mapping table if you wish; they keys
are the host name suffixes and the values are the `routing_domain`s that should
be applied.

## IP Based Rollup

!!! info
    This technique has a number of caveats and is not generally recommended
    unless you have no other choice.

This technique analyzes the first (lowest preferenced / highest priority) MX
record for the destination domain. If it matches a hostname suffix in the
provided mapping table it is queued into a special `.ip_rollup` queue specified
by the mapping.

Messages in that queue will be relayed to the set of IP addresses from that
first MX host name.

All domains that have the same matching MX hostname suffix and set of IP
addresses will egress through the ready queue and be subject to the same
shaping rules.

```lua
local rollup = require 'policy-extras.rollup'

kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local params = {}
  rollup.apply_ip_rollup_to_queue_config(domain, routing_domain, params)
  return kumo.make_queue_config(params)
end)

kumo.on('smtp_server_message_received', function(msg)
  rollup.reroute_using_ip_rollup(msg, {
    ['.mail.protection.outlook.com.'] = 'outlook.ip_rollup',
  })
end)
```

You would configure shaping rules with this in your `shaping.toml`:

```toml
["outlook.ip_rollup"]
mx_rollup = false
# shaping parameters here
```

With above configuration in place, for a hypothetical `foo.com` domain whose
MX and A records look like:

```console
$ dig +short mx foo.com
1 foo-com.mail.protection.outlook.com.
$ dig +short a foo-com.mail.protection.outlook.com.
104.47.24.36
104.47.25.36
```

what will happen is:

* The message will be queued to a *scheduled queue* named `foo.com!outlook.ip_rollup`
* The ready queue that it will use to egress will be named `mx_list:[104.47.24.36],[104.47.25.36]`.

For some other domain, for example, a hypothetical `bar.com` domain whose
MX and A records look like:

```console
$ dig +short mx bar.com
1 bar-com.mail.protection.outlook.com.
$ dig +short a bar-com.mail.protection.outlook.com.
104.47.24.36
104.47.25.36
```

what will happen is:

* The message will be queued to a *scheduled queue* named `bar.com!outlook.ip_rollup`
* The ready queue that it will use to egress will be named `mx_list:[104.47.24.36],[104.47.25.36]`.

In this scenario, both `foo.com` and `bar.com` will egress through the same
ready queue and have the shaping from the `outlook.ip_rollup` section of your
`shaping.toml` applied across them both.

There are a number of important caveats with this particular IP rollup approach:

* We will never try anything beyond the highest priority MX for the matching domains
* The IP addresses for the destination domains can vary over time. The
  scheduled queue name (`foo.com!outlook.ip_rollup`) will remain the same, but
  the list of IP addresses in the ready queue name will adjust to match the
  evolving DNS. That means that during a transition there may be multiple ready
  queues with different names as the IPs rotate.  This can reduce the efficacy
  of the rollup, but is a necessary function in order to handle the situation
  where the destination site has an outage and we need to stop using the old IPs.
* If the destination site is using round-robin DNS for load balancing purposes,
  this approach is not useful as the resulting `mx_list` in the site name
  will vary too frequently for there to be a meaningful or useful rolling up
  of the egress.
* Since the ready queue names look like `mx_list:[IP],[IP]` it can be hard to intuit
  from a simple glance where that mail is going.

With the above in mind, we recommend using this particular IP rollup implementation
**only as a last resort**.

