local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
local typing = require 'policy-extras.typing'
local Bool, List, Map, Option, Record, String =
  typing.boolean,
  typing.list,
  typing.map,
  typing.option,
  typing.record,
  typing.string

local function is_listener_domain_option(name, value)
  local p = { [name] = value }
  local status, err = pcall(kumo.make_listener_domain, p)
  if not status then
    local err = typing.extract_deserialize_error(err)
    if tostring(err):find 'invalid type' then
      return false, err
    end
    return false, nil
  end
  return status
end

local ListenerConfig = Record('ListenerConfig', {
  _dynamic = is_listener_domain_option,
  relay_from_authz = Option(List(String)),
})

local DomainToConfig = Map(String, ListenerConfig)
local ListenerDomainConfig = Map(String, DomainToConfig)

local function process_loaded_data(data, target)
  local by_listener = ListenerDomainConfig(data.listener or {})
  data.listener = nil
  for domain, params in pairs(data) do
    if not by_listener['*'] then
      by_listener['*'] = DomainToConfig {}
    end
    by_listener['*'][domain] = params
  end

  for listener, entries in pairs(by_listener) do
    for domain, params in pairs(entries) do
      if not target[listener] then
        target[listener] = DomainToConfig {}
      end
      if not target[listener][domain] then
        target[listener][domain] = ListenerConfig {}
      end

      for k, v in pairs(params) do
        target[listener][domain][k] = v
      end
    end
  end

  -- print(kumo.serde.json_encode_pretty(target))
end

local function load_data_from_file(file_name, target)
  local data = utils.load_json_or_toml_file(file_name)
  return process_loaded_data(data, target)
end

-- compile the domain lookups
local function compile_data(by_listener)
  local compiled = {}
  for listener, mapping in pairs(by_listener) do
    compiled[listener] = kumo.domain_map.new(mapping)
  end

  return compiled
end

local function load_data(data_files)
  local by_listener = {}
  for _, file_name in ipairs(data_files) do
    load_data_from_file(file_name, by_listener)
  end

  return compile_data(by_listener)
end

local function parse_toml_data(toml_text)
  local data = kumo.toml_parse(toml_text)
  local by_listener = {}
  process_loaded_data(data, by_listener)
  print('compiling', kumo.json_encode_pretty(by_listener))
  return compile_data(by_listener)
end

local function lookup_impl(
  by_listener,
  domain_name,
  listener,
  conn_meta,
  skip_make
)
  local listener_map = by_listener[listener]
  if listener_map then
    local listener_domain = listener_map[domain_name]
    if listener_domain then
      local relay_from_authz = listener_domain.relay_from_authz
      -- Don't try and pass relay_from_authz into make_listener_domain
      listener_domain.relay_from_authz = nil

      if
        relay_from_authz
        and utils.table_contains(
          relay_from_authz,
          conn_meta:get_meta 'authz_id'
        )
      then
        if not listener_domain.relay_from then
          listener_domain.relay_from = {}
        end
        -- Add the peer to the relay_from list
        local peer_ip, _peer_port =
          utils.split_ip_port(conn_meta:get_meta 'received_from')
        listener_domain.relay_from = { peer_ip }
      end

      if skip_make then
        return ListenerConfig(listener_domain)
      end
      return kumo.make_listener_domain(listener_domain)
    end
  end

  return nil
end

local function get_listener_domain_impl(
  by_listener,
  domain_name,
  listener,
  conn_meta,
  skip_make
)
  local result =
    lookup_impl(by_listener, domain_name, listener, conn_meta, skip_make)
  if result then
    return result
  end
  -- Now try that domain against the '*' listener entry
  local result =
    lookup_impl(by_listener, domain_name, '*', conn_meta, skip_make)
  if result then
    return result
  end
  -- Now try the '*' domain and '*' listener
  return lookup_impl(by_listener, '*', '*', conn_meta, skip_make)
end

--[[
Usage:

Create a `/opt/kumomta/etc/listener_domains.toml` file with
contents like:

```toml
["example.com"]
# allow relaying mail from anyone, so long as it is
# addressed to example.com
relay_to = true

["bounce.example.com"]
# accept and log OOB bounce reports sent to bounce.example.com
log_oob = true

["fbl.example.com"]
# accept and log ARF feedback reports sent to fbl.example.com
log_arf = true

["send.example.com"]
# relay to anywhere, so long as the sender domain is send.example.com
# and the connected peer matches one of the listed CIDR blocks
relay_from = [ '10.0.0.0/24' ]

["auth-send.example.com"]
# relay to anywhere, so long as the sender domain is auth-send.example.com
# and the connected peer has authenticated as any of the authorization identities
# listed below using SMTP AUTH
relay_from_authz = [ 'username1', 'username2' ]

# wildcards are permitted. This will match
# <anything>.example.com that doesn't have
# another non-wildcard entry explicitly
# listed in this set of domains.
# Note that "example.com" won't match
# "*.example.com".
["*.example.com"]
# You can specify multiple options if you wish
log_oob = true
log_arf = true
relay_to = true

# and you can explicitly set options to false to
# essentially exclude an entry from a wildcard
["www.example.com"]
relay_to = false
log_arf = false
log_oob = false

# Define a per-listener configuration
[listener."127.0.0.1:25"."*.example.com"]
log_oob = false
```

Then in your policy:

```
local listener_domains = require 'policy-extras.listener_domains'

kumo.on('get_listener_domain', listener_domains:setup({'/opt/kumomta/etc/listener_domains.toml'}))
```


You can use multiple data files, and they can be either toml
or json so long as they have the structure described above;
the keys are domain names with optional wildcard domains
being supported.  The contents of a section are any valid
key/value pair supported by kumo.make_listener_domain; the
section value will be passed to kumo.make_listener_domain.

]]
function mod:setup(data_files)
  if mod.CONFIGURED then
    error 'listener_domains module has already been configured'
  end

  local cached_load_data = kumo.memoize(load_data, {
    name = 'listener_domains_data',
    ttl = '5 minutes',
    capacity = 10,
    invalidate_with_epoch = true,
  })

  local function get_listener_domain(domain_name, listener, conn_meta)
    local by_listener = cached_load_data(data_files)
    return get_listener_domain_impl(
      by_listener,
      domain_name,
      listener,
      conn_meta
    )
  end

  mod.CONFIGURED = {
    data_files = data_files,
  }

  return get_listener_domain
end

kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local failed = false

  function show_context()
    if failed then
      return
    end
    failed = true
    kumo.validation_failed()
    print 'Issues found in the combined set of listener_domain files:'
    for _, file_name in ipairs(mod.CONFIGURED.data_files) do
      if type(file_name) == 'table' then
        print ' - (inline table)'
      else
        print(string.format(' - %s', file_name))
      end
    end
  end

  local status, err = pcall(load_data, mod.CONFIGURED.data_files)
  if not status then
    show_context()
    print(err)
  end
end)

function mod:test()
  local cached_load_data = kumo.memoize(load_data, {
    name = 'test_listener_domains_data',
    ttl = '5 minutes',
    capacity = 10,
  })

  local function get_listener_domain(
    data,
    domain_name,
    listener,
    conn_meta,
    skip_make
  )
    local by_listener = cached_load_data { data }
    return get_listener_domain_impl(
      by_listener,
      domain_name,
      listener,
      conn_meta,
      skip_make
    )
  end

  local open_relay = [=[
['*']
relay_to = true

['somewhere.com']
relay_to = false
relay_from = ['10.0.0.0/24']

# Define a per-listener configuration
[listener."127.0.0.1:25"."*.example.com"]
log_oob = false

['elsewhere.com']
relay_to = false
relay_from_authz = ["john"]

]=]

  local data = kumo.serde.toml_parse(open_relay)

  -- Fake up something that quacks like the connection metadata object.
  -- This is really just a matter of adding a metatable with a get_meta
  -- method to it.
  local function make_conn_meta(obj)
    local methods = {}
    function methods:get_meta(key)
      return rawget(self, key)
    end
    local mt = {
      __index = methods,
    }
    setmetatable(obj, mt)
    return obj
  end

  utils.assert_eq(
    get_listener_domain(
      data,
      'example.com',
      '127.0.0.1:25',
      make_conn_meta {}
    ),
    kumo.make_listener_domain { relay_to = true }
  )

  utils.assert_eq(
    get_listener_domain(
      data,
      'woof.example.com',
      '127.0.0.1:25',
      make_conn_meta {}
    ),
    kumo.make_listener_domain { log_oob = false, relay_to = true }
  )

  utils.assert_eq(
    get_listener_domain(
      data,
      'somewhere.com',
      '127.0.0.1:25',
      make_conn_meta { received_from = '10.0.0.1' }
    ),
    kumo.make_listener_domain {
      relay_from = { '10.0.0.0/24' },
      relay_to = false,
    }
  )

  utils.assert_eq(
    get_listener_domain(
      data,
      'elsewhere.com',
      '10.0.0.1:25',
      make_conn_meta {}
    ),
    kumo.make_listener_domain { relay_to = false }
  )

  utils.assert_eq(
    get_listener_domain(
      data,
      'elsewhere.com',
      '10.0.0.1:25',
      make_conn_meta { authz_id = 'john', received_from = '10.0.0.1:25' }
    ),
    kumo.make_listener_domain { relay_from = { '10.0.0.1' } }
  )

  local status, err = pcall(
    parse_toml_data,
    [[
# Define a per-listener configuration
[listener."127.0.0.1:25"."*.example.com"]
log_arf = "yes"
  ]]
  )
  assert(not status)
  utils.assert_matches(
    err,
    'ListenerConfig: invalid value for field \'log_arf\'\n.*invalid type: string "yes", expected a boolean'
  )
end

return mod
