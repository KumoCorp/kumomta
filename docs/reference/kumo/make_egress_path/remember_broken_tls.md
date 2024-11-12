# remember_broken_tls

{{since('2024.11.08-d383b033')}}

While many destination sites on the internet advertise support for STARTTLS, a
subset of them are problematic for reasons that can include:

 * Running ancient crypto software with deprecated or otherwise incompatible
   cipher suites
 * Misconfigured CN
 * Expired certificates

These can cause issues when it comes to reliably enabling TLS for a given
session when running in `Opportunistic` TLS mode; since the broken TLS can
prevent re-using the existing session in clear text we can end up failing
to connect to any of the candidate hosts for a given site.

That is where this option comes into play: when it is set to a duration
string, that will cause `kumod` to remember that a given site has broken
TLS for up to that duration.

Subsequent connection attempts will use that information to influence how
it should proceed; for `Opportunistic` modes we will treat the session
as if STARTTLS was not advertised.  For `Required` modes we will log
an error that mentions that `remember_broken_tls` is set.

```lua
kumo.make_egress_path {
  remember_broken_tls = '3 days',
}
```

!!! note
    This information is cached locally in the memory of a given kumod
    process.  It is not shared with other nodes in a cluster, and it
    will be forgotten when the node is restarted.
