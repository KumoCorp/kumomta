# trust_anchor_file

{{since('2025.05.06-b29689af')}}

Configures the DNSSEC trust anchor used for [validation](validate.md). It takes
one of two forms.

## Static anchor file

A string naming a file that contains one or more `DS` or `DNSKEY` records:

```lua
options = {
  validate = true,
  trust_anchor_file = '/etc/kumomta/root-anchors.txt',
}
```

The file is read once at startup and is never updated, so you are responsible
for keeping it current across root KSK rollovers. On the unbound backend it is
loaded in addition to the bundled root anchors (when validation is enabled); on
the hickory backend it replaces them.

## Managed anchor file (RFC 5011)

{{since('dev')}}

A table of the form `{ managed = "<path>" }` names a file that is automatically
maintained according to [RFC 5011](https://www.rfc-editor.org/rfc/rfc5011): the
resolver tracks root KSK rollovers and rewrites the file as the keys change, so
it stays current without operator intervention or kumomta upgrades. See ICANN's
[Root Zone KSK Rollover](https://www.icann.org/resources/pages/ksk-rollover)
page for background on when rollovers happen and what operators should check.

```lua
options = {
  trust_anchor_file = { managed = '/var/lib/kumomta/root.key' },
}
```

This form is supported by the **unbound backend only**; configuring it with the
hickory backend is a configuration-time error.

The file must be writable by kumomta. If it does not yet exist (or is empty),
kumomta seeds it with the bundled current root anchors so that validation works
immediately, and RFC 5011 maintenance takes over from there. You may instead
seed it yourself, for example with
[unbound-anchor](https://www.nlnetlabs.nl/documentation/unbound/unbound-anchor/).

## Default

For most deployments, leaving this unset and setting `validate = true` is
sufficient: both backends validate against root anchors bundled with kumomta,
which are refreshed when you upgrade. The bundled anchors only go stale after a
future root KSK rollover, until you either upgrade kumomta or switch to a
managed anchor file.
```
