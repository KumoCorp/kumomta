# Configuration Concepts

KumoMTA differs from existing commercial and Open Source MTAs in that there is no configuration file in the traditional sense. Instead, all server configuration is achieved through the passing of a policy file written in Lua at server startup.

At first, configuration using a policy script may seem like a departure from the traditional approach to configuration, but using a Lua script as a configuration methodology will look quite familiar to administrators of the popular commercial MTA solutions.

Take a look at the [example policy](example.md) to see how a configuration policy approach can be quite similar to a traditional configuration file.

For more information on implementing policies in KumoMTA, refer to the [policy chapter](../policy/index.md).

## Configuration Location

By default, the server will load from `/opt/kumomta/policy/init.lua` on startup.

## Configuration Structure

There is a lot of flexibility in how a KumoMTA policy file is laid out, but a few things are generally consistent:

### Init Event

The majority of the base server configuration will reside within an init event handler. The init event is fired when the server first starts up, making it the appropriate time for basic server configuration.

Because these attributes are only loaded on init, an explicit reload must be triggered when anything in the init handler is changed, whether the change is in the policy script itself or a change in a data source or file accessed by the policy script.

An example:

```lua
kumo.on('init', function()
  kumo.define_spool {
    name = 'data',
    path = '/var/tmp/kumo-spool/data',
    kind = 'RocksDB',
  }
end)
```

### Realtime Events

Attributes that are needed at the time of queueing and sending are handled in events that are called repeatedly as messages pass through the server. Any such events are constantly firing, and as such any file or data source access involved in those events will update immediately unless caching is configured.

That said, any modification to the policy script itself is subject to caching of the lua policy, which is refreshed every 300 seconds or 1024 executions by default.

An example:

```lua
kumo.on('get_queue_config', function(domain, tenant, campaign)
  return kumo.make_queue_config {
    egress_pool = tenant,
  }
end)
```

### External Data

Because the configuration is implemented through policy, the traditional practice of breaking things up into discrete files and assembling them using include statements is not used.

Since includes were often used for dynamic information such as relay domains or relay hosts, the recommended practice is to store that specific data in a data file or data source and load it using Lua data access functions.

For example, DKIM signing information by domain and selector could be stored in a JSON file like this:

```json
    [{"examplecorp.com": "s1024"} ,{"newcorp.com" "dkim2023"}]
```

The data file could then be read and used to control signing:

```lua
local DKIM_CONFIG = kumo.json_load '/opt/kumomta/policy/dkim_config.json'

function dkim_sign(msg)
  local sender_domain = msg:from_header().domain
  local selector = DKIM_CONFIG[sender_domain]
  -- and so on
end
```
