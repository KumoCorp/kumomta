# remember_broken_tls

{{since('2024.11.08-d383b033')}}

While many destination sites on the internet advertise support for STARTTLS, a
subset of them are problematic for reasons that can include:

 * Running ancient crypto software with deprecated or otherwise incompatible
   cipher suites
 * Misconfigured CN
 * Expired certificates

In `Opportunistic` TLS mode, a broken STARTTLS handshake fails that
connection attempt, and every subsequent attempt to the same host repeats
the same failed handshake. If every candidate host for a site has broken
TLS, we end up failing to connect to any of them.

That is where this option comes into play: when it is set to a duration
string, that will cause `kumod` to remember that a given site has broken
TLS for up to that duration.

For subsequent connections in `Opportunistic` or `OpportunisticInsecure`
mode, a remembered failure causes STARTTLS to be treated as not advertised.
`Required` and `RequiredInsecure` never fall back to plaintext, regardless
of this memory.

Whether a TLS failure is remembered depends on the effective `enable_tls`
for the message being delivered, after DANE and MTA-STS are applied:

|Effective `enable_tls` when TLS fails|Earlier versions|{{since('dev', inline=True)}}|
|-------------------------------------|----------------|-----------------------------|
|`Opportunistic` or `OpportunisticInsecure`|Later opportunistic connections to the site skip STARTTLS|Later opportunistic connections to the site skip STARTTLS|
|`Required` or `RequiredInsecure`|Later opportunistic connections to the site skip STARTTLS|Later connections still attempt STARTTLS|

For example, if DANE authentication fails for one domain on a shared site,
messages for other domains on that site keep using TLS.

The same applies to the 15-minute memory enabled by
[opportunistic_tls_reconnect_on_failed_handshake](opportunistic_tls_reconnect_on_failed_handshake.md).
A failure that is not remembered does not clear one that already is.

```lua
kumo.make_egress_path {
  remember_broken_tls = '3 days',
}
```

!!! note
    This information is cached locally in the memory of a given kumod
    process.  It is not shared with other nodes in a cluster, and it
    will be forgotten when the node is restarted.
