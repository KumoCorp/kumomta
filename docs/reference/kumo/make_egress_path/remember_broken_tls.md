# remember_broken_tls

{{since('2024.11.08-d383b033')}}

While many destination sites on the internet advertise support for STARTTLS, a
subset of them are problematic for reasons that can include:

 * Running ancient crypto software with deprecated or otherwise incompatible
   cipher suites
 * Misconfigured CN
 * Expired certificates

These can prevent delivery in an opportunistic TLS mode. A failed handshake
closes the connection rather than resuming SMTP in plaintext on that socket.
`OpportunisticInsecure` retries the same address once on a fresh plaintext
connection, but that retry alone does not make later connections skip TLS.
Without broken-TLS memory or a permitted fresh-connection fallback, repeated
failures can exhaust the candidate hosts for a site.

That is where this option comes into play: when it is set to a duration
string, that will cause `kumod` to remember that a given site has broken
TLS for up to that duration.

Subsequent connection attempts will use that information to influence how
it should proceed; for `Opportunistic` modes we will treat the session
as if STARTTLS was not advertised. For `Required` modes, TLS is still required
regardless of this memory.

```lua
kumo.make_egress_path {
  remember_broken_tls = '3 days',
}
```

!!! note
    This information is cached locally in the memory of a given kumod
    process.  It is not shared with other nodes in a cluster, and it
    will be forgotten when the node is restarted.
