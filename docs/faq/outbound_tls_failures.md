---
description: "Handle TLS handshake and certificate failures to a destination — OpportunisticInsecure, remember_broken_tls, UnknownIssuer, and MTA-STS errors."
---

# How Do I Handle TLS Handshake or Certificate Failures to a Destination?

Some receiving servers present broken, mismatched, or expired TLS, and delivery to them fails repeatedly with errors such as `BadSignature`, `NotValidForName`, `UnknownIssuer`, or `received fatal alert: UnexpectedMessage`. What to do depends on whose problem it is: the remote server's TLS, or your own trust configuration.

## A remote server with broken or mismatched STARTTLS

If a specific destination's STARTTLS is broken or its certificate does not match its hostname, you can configure KumoMTA to remember the failure and fall back to cleartext for that host instead of failing every attempt. Set these on the egress path (typically via your shaping file):

```toml
["broken-tls-destination.example"]
enable_tls = "OpportunisticInsecure"
remember_broken_tls = "1 hour"
opportunistic_tls_reconnect_on_failed_handshake = true
```

* `enable_tls = "OpportunisticInsecure"` attempts TLS but does not require a valid certificate.
* `remember_broken_tls` makes KumoMTA remember a host's broken TLS for the given duration and skip straight to cleartext during that window.
* `opportunistic_tls_reconnect_on_failed_handshake = true` lets KumoMTA reconnect without TLS within the same delivery attempt when the handshake fails.

## Your own outbound trust failures (UnknownIssuer)

If outbound delivery fails with `UnknownIssuer` for many destinations at once, KumoMTA likely cannot find a CA bundle. Install the system `ca-certificates` package and run a current release; recent versions use the system trust store.

## Intermittent "554 TLS required" behind one NAT IP

When a single source IP fronts several backends with inconsistent TLS (common with some security appliances), you may see intermittent `554 TLS required`. Rather than `remember_broken_tls`, rewrite the response so the message retries and succeeds on a later attempt:

```lua
kumo.on('smtp_client_rewrite_delivery_status', function(response, domain, ...)
  if response:find '554' and response:find 'TLS required' then
    return 454 -- demote to transient so it retries
  end
end)
```

## MTA-STS enforcement (Gmail, Outlook)

Errors referencing an MTA-STS policy are the provider correctly refusing insecure delivery. The fix is to make your TLS to that provider valid, not to disable enforcement.

!!! tip
    Debug TLS negotiation with `kcli trace-smtp-client` and a temporary verbose log filter (`kcli set-log-filter 'kumod=debug'`).

## See also

* [make_egress_path / enable_tls](../reference/kumo/make_egress_path/enable_tls.md)
* [make_egress_path / remember_broken_tls](../reference/kumo/make_egress_path/remember_broken_tls.md)
* [smtp_client_rewrite_delivery_status](../reference/events/smtp_client_rewrite_delivery_status.md)
