# relay_to

Optional boolean. Defaults to `false`. When set to `true`, allows relaying mail
*from anyone*, so long as it is addressed to the requested domain.

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if domain == 'example.com' then
    return kumo.make_listener_domain {
      relay_to = true,
    }
  end
end)
```

### Enable relaying based on SMTP authentication

This example shows how to use the connection metadata information to determine
if the current session is authenticated, and use that to govern whether
relaying is allowed.

In this example, any user is permitted to relay to anywhere:

```lua
kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  if conn_meta:get_meta 'authz_id' then
    -- We're authenticated as someone.
    -- Allow relaying
    return kumo.make_listener_domain {
      relay_to = true,
    }
  end
end)
```

A more sophisticated configuration might use a mapping to control
which domains are allowed relaying based on the authorization id:

```lua
local RELAY_BY_DOMAIN = {
  ['example.com'] = {
    -- The user scott is permitted to relay to example.com
    ['scott'] = true,
  },
}

kumo.on('get_listener_domain', function(domain, listener, conn_meta)
  local dom_map = RELAY_BY_DOMAIN[domain]
  if dom_map then
    local authz_id = conn_meta:get_meta 'authz_id'
    if dom_map[authz_id] then
      return kumo.make_listener_domain {
        relay_to = true,
      }
    end
  end
end)
```


