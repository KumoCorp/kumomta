# Advanced Tenant to Pool Mapping

if you want to verify that the tenant is a known-good tenant then I'd recommend handling that in the smtp_server_message_received event:

```lua
local tenant = msg:get_first_named_header_value 'X-tenant'
if not tenant then
  kumo.reject(500, 'missing x-tenant header')
end

local TENANT_TO_POOL = {
  ['tenant-id-0'] = 'pool-0',
  ['tenant-id-1'] = 'pool-0',
  ['tenant-id-2'] = 'pool-1',
}

if not TENANT_TO_POOL[tenant] then
  kumo.reject(500, 'invalid/unknown tenant ' .. tenant)
end

msg:set_meta('tenant', tenant)
```

You could then reference that same TENANT_TO_POOL mapping later on:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local params = {
    max_age = '5 minutes',
    retry_interval = '10 minutes',
    max_retry_interval = '100 minutes',
    egress_pool = TENANT_TO_POOL[tenant], -- here
  }
  merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)
```

TENANT_TO_POOL could also be a json file, sqlite db and so on

more robust tenant validation should probably consider the sending domain as well.  Maybe the data looks

```lua
local TENANTS = {
  ['tenant-id-0'] = {
    domain_to_pool = {
      ['tenant-0.com'] = 'pool-0',
    },
  },
}
```

and the policy should check the msg:sender().domain as well as the tenant before accepting the message

if you want to allow leaving the pool unspecified then:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign, routing_domain)
  local params = {
    max_age = '5 minutes',
    retry_interval = '10 minutes',
    max_retry_interval = '100 minutes',
    egress_pool = TENANT_TO_POOL[tenant], -- HERE
  }
  merge_into(TENANT_PARAMS[tenant] or {}, params)
  return kumo.make_queue_config(params)
end)
```

the HERE bit will evaluate to nil if there is no valid mapping, which is equivalent to leaving the pool unspecified, which is valid: not special source configuration will be used in that case, and we'll use whatever IP the kernel chooses when we try to connect

if you want to use an explicitly configured pool instead of the default unspecified behavior, then you can do:

```lua
egress_pool = TENANT_TO_POOL[tenant] or 'my-fallback-pool-name'
```
