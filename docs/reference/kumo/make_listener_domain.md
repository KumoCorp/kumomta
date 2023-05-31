# `kumo.make_listener_domain {PARAMS}`

Make a listener-domain configuration object.

The [get_listener_domain](../events/get_listener_domain.md) event expects
one of these to be returned to it (or a `nil` value).

A listener-domain contains information that affects whether an incoming
SMTP message will be accepted and/or relayed.

By default, unless the client is connecting from one of the `relay_hosts`,
relaying is denied.

`PARAMS` is a lua table that can accept the keys listed below.

## relay_to

Optional boolean. Defaults to `false`. When set to `true`, allows relaying mail
*from anyone*, so long as it is addressed to the requested domain.

```lua
kumo.on('get_listener_domain', function(domain)
  if domain == 'example.com' then
    return kumo.make_listener_domain {
      relay_to = true,
    }
  end
end)
```

## log_oob

Optional boolean. Defaults to `false`. When set to `true`, if the incoming mail
is an out-of-band (OOB) bounce report formatted according to RFC 3464, and is
addressed to the requested domain, the message will be accepted and logged to
the delivery logs.

```lua
kumo.on('get_listener_domain', function(domain)
  if domain == 'bounce.example.com' then
    return kumo.make_listener_domain {
      log_oob = true,
    }
  end
end)
```

## log_arf

Optional boolean. Defaults to `false`. When set to `true`, if the incoming mail
is an ARF feedback report formatted according to RFC 5965, and is addressed to
the requested domain, the message will be accepted and logged to the delivery
logs.

```lua
kumo.on('get_listener_domain', function(domain)
  if domain == 'fbl.example.com' then
    return kumo.make_listener_domain {
      log_arf = true,
    }
  end
end)
```

## relay_from

Optional CIDR list. Defaults to an empty list. If the connected client is from
an IP address that matches the CIDR list, and the sending domain matches the
requested domain, then relaying will be allowed.

```lua
kumo.on('get_listener_domain', function(domain)
  if domain == 'send.example.com' then
    return kumo.make_listener_domain {
      relay_from = { '10.0.0.0/24' },
    }
  end
end)
```

