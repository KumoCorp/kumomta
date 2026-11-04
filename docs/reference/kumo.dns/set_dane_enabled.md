# set_dane_enabled

```lua
kumo.dns.set_dane_enabled(ENABLED)
```

{{since('dev')}}

Controls whether MX resolution probes
[DANE](https://datatracker.ietf.org/doc/html/rfc7672) `TLSA` records to
influence the *site_name* for a domain. `ENABLED` is a boolean. The default
value is `false`.

To enable DANE-aware site names:

```lua
kumo.on('pre_init', function()
  kumo.dns.set_dane_enabled(true)
end)
```

This is distinct from the per-egress-path
[enable_dane](../kumo/make_egress_path/enable_dane.md) option, which governs
whether DANE is enforced when connecting. In short:

* `kumo.dns.set_dane_enabled`: whether resolution probes `TLSA` records and lets
  them modify the *site_name*.
* `enable_dane` (egress path): whether DANE authenticates the peer when
  connecting.

## Why this is a global option

Resolution assigns the *site_name* before the egress path is selected, so no
per-path `enable_dane` value exists yet for it to read.

## How DANE influences the *site_name*

A domain is given a distinct *site_name* only when its own MX selection was not
DNSSEC-secure, yet one of its MX hosts publishes DANE `TLSA` records. Such a
domain resolves to a *site_name* prefixed with `[dane-insecure]`. Since the
*site_name* is the ready-queue key, the prefixed domain gets a ready queue of
its own.

With per-egress-path [enable_dane](../kumo/make_egress_path/enable_dane.md)
enabled, connection setup and reuse enforce each message's DANE requirements
whether or not this naming option is enabled. Separating the queues allows
each group to keep its own connections, egress-path configuration, limits
and backoff. It can reduce connection replacements when traffic alternates
between domains requiring DANE and PKIX authentication.

The remaining cases are left untagged and keep sharing the common ready queue:

* A domain whose MX selection is DNSSEC-secure is never tagged.
* An insecure domain whose MX hosts don't publish `TLSA` records at all (the
  common case, including domains hosted on providers that do not deploy DANE) is
  never tagged.
* If the `TLSA` lookup cannot be completed, the domain is left untagged rather
  than split off.

Like DANE enforcement itself, this requires a DNSSEC-validating resolver. See
[enable_dane](../kumo/make_egress_path/enable_dane.md). Without one, `TLSA`
records never validate as secure and no domain is ever tagged.
