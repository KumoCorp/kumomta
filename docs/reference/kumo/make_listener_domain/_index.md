# `kumo.make_listener_domain {PARAMS}`

Make a listener-domain configuration object.

The [get_listener_domain](../../events/get_listener_domain.md) event expects
one of these to be returned to it (or a `nil` value).

A listener-domain contains information that affects whether an incoming
SMTP message will be accepted and/or relayed.

By default, unless the client is connecting from one of the `relay_hosts`,
relaying is denied.

`PARAMS` is a lua table that can accept the keys listed below.

