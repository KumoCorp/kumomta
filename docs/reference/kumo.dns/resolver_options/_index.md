# Resolver Options

{{since('dev')}}

This section documents the fields accepted in the `options` table passed to
[kumo.dns.configure_resolver](../configure_resolver.md),
[kumo.dns.configure_unbound_resolver](../configure_unbound_resolver.md),
[kumo.dns.define_resolver](../define_resolver.md), and the table returned
by [kumo.dns.load_resolv_conf](../load_resolv_conf.md).

In earlier releases the `options` table accepted hickory DNS's own option
names directly; see the
[version table in configure_resolver](../configure_resolver.md) for links to
the relevant hickory documentation per KumoMTA release. KumoMTA now defines
these names itself; the supported set is documented in the pages below.

All fields are optional. Unknown fields are a configuration-time error.
The Unbound backend only accepts a small subset of these options and will
error on the rest at configuration time; see
[configure_unbound_resolver](../configure_unbound_resolver.md) for the
supported set.

## Resolver Options { data-search-exclude }
