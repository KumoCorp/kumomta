# set_mta_sts_enabled

```lua
kumo.dns.set_mta_sts_enabled(ENABLED)
```

{{since('dev')}}

Controls whether MX resolution consults [MTA-STS](https://datatracker.ietf.org/doc/html/rfc8461)
policies. `ENABLED` is a boolean. The default value is `true`.

```lua
kumo.on('pre_init', function()
  kumo.dns.set_mta_sts_enabled(true)
end)
```

This option controls whether MTA-STS influences the *site_name* for a domain.
When enabled, the MTA-STS policy published by a destination domain is evaluated
against that domain's own MX records as part of resolving its site, before
several domains that share the same MX records are rolled up (and thus before we
know the egress path name). This is distinct from the per-egress-path
[enable_mta_sts](../kumo/make_egress_path/enable_mta_sts.md) option, which
governs whether a resolved policy is allowed to raise the connection's TLS
posture. In short:

* `kumo.dns.set_mta_sts_enabled` — do we query MTA-STS records and allow them to
  modify the *site_name*?
* `enable_mta_sts` (egress path) — do we let a resolved policy require TLS?

## Why this is a global option

MTA-STS policy is a property of the destination *routing domain*, but several
routing domains that share the same set of MX records are rolled up into a
single ready queue keyed by their shared *site_name*. Evaluating the policy
during resolution — before that rollup, and thus before we know the egress path
name — keeps each domain's policy scoped to its own resolution, so a
misconfigured domain cannot affect other domains sharing the same MX hosts.

## How MTA-STS influences the *site_name*

The effect on the *site_name* depends on the policy that the destination domain
publishes:

* **No MTA-STS policy** (the common case): resolution is unaffected and the
  *site_name* is derived from the MX records as usual.

* **A policy whose allowed MX patterns cover all of the domain's MX hosts** (a
  correctly specified policy): resolution is unaffected and the *site_name* is
  unchanged. The policy's TLS requirement may still apply on egress paths that
  opt in via `enable_mta_sts`.

* **A policy that covers only some of the MX hosts**: the disallowed hosts are
  removed during resolution, so the *site_name* reflects only the permitted
  hosts. Such a domain rolls up only with other domains that resolve to the same
  permitted set.

* **A policy that matches none of the domain's MX hosts** (a misconfiguration):
  the domain is treated as undeliverable. It fails to resolve and self-isolates,
  rather than joining the shared site used by other domains with the same MX
  records.

For example, if `random-domain-example.com` shares iCloud's MX records:

```console
$ dig +short mx random-domain-example.com
10 mx01.mail.icloud.com.
10 mx02.mail.icloud.com.
```

but publishes an MTA-STS policy that can never match those hosts:

```console
$ curl https://mta-sts.random-domain-example.com/.well-known/mta-sts.txt
version: STSv1
mode: enforce
mx: *.mx.cloudflare.net
max_age: 86400
```

then resolution fails with a transient error and its messages are delayed (and
ultimately expire per your retry schedule). You will see a log record similar
to:

```
MTA-STS enforce policy for random-domain-example.com permits none of its MX
hosts ["mx01.mail.icloud.com.", "mx02.mail.icloud.com."]; allowed mx patterns:
["*.mx.cloudflare.net"]. The destination is undeliverable until its MTA-STS
policy is corrected.
```

This is intentional and correct: the destination's published policy says its
mail must not be delivered to those hosts. **The right fix is for the
destination domain to correct its broken MTA-STS policy.** Forcing delivery
anyway deliberately ignores a published security policy.

## Forcing delivery to a broken domain (not recommended)

If a particular domain is important enough that you want to deliver to it
despite a broken policy, you can route it through an explicit `mx_list` in
[get_queue_config](../kumo/make_queue_config/protocol.md). Configuring an
`mx_list` bypasses `MailExchanger` resolution entirely and places the domain in
its own dedicated queue:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign)
  if domain == 'random-domain-example.com' then
    -- Look up a CLEAN domain that shares the same MX records (e.g. the
    -- provider's canonical domain). Do NOT look up the broken domain
    -- itself -- it would fail to resolve for the same reason.
    local mx = kumo.dns.lookup_mx 'icloud.com'
    return kumo.make_queue_config {
      protocol = {
        smtp = {
          mx_list = mx.hosts,
        },
      },
    }
  end
  -- ...normal config...
end)
```
