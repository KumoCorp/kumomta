# `redis.open { PARAMS }`

Opens a connection to a [Redis](https://redis.io/) data store and returns a connection handle.

```lua
local redis = require 'redis'

-- Open a connection and increment a counter, returning its new value.
-- Redis URL syntax is: 'redis://[<username>][:<password>@]<hostname>[:port][/[<db>][?protocol=<protocol>]]'
local conn = redis.open { node = 'redis://127.0.0.1/' }
print(conn:query('incr', 'test-count'))
```

*PARAMS* is a lua table with the following keys:

* `node` - the redis URL string identifying the server.  Can be a table listing
  multiple servers if you have a redis cluster deployed.  Redis URL strings
  have the form `redis://[<username>][:<password>@]<hostname>[:port][/[<db>][?protocol=<protocol>]]`

* `pool_size` - optional integer. Specifies the maximum number of spare
  connections to be maintained by the connection pool.  The default is 10.

* `read_from_replicas` - optional boolean. If true, when connecting to a redis
  cluster, reads are issued to replicas while writes are issued to the primary
  data stores.

* `username` - optional string. convenience for working with the cluster, so
  that you only need to specify the username once. This is not used for connecting
  to a single node.

* `password` - optional string. convenience for working with the cluster, so
  that you only need to specify the password once. This is not used for connecting
  to a single node.

* `connect_timeout` - optional duration string. Specify how long to keep attempting
  to connect to redis. The default is `30 seconds`. {{since('2024.06.10-84e84b89', inline=True)}}

* `wait_timeout` - optional duration string. Specify how long to wait for an
  available connection. If the pool is at capacity (`pool_size`), then an `open`
  call will block for up this duration for a connection to be released and made 
  to connect to redis. The default is `30 seconds`. {{since('2024.09.02-c5476b89', inline=True)}}

* `response_timeout` - optional duration string. Specify how long to wait for the
  response to a query.  {{since('2024.09.02-c5476b89', inline=True)}}

* `recycle_timeout` - optional duration string. Specify how long to wait for the
  liveness check when testing whether a connection can be resused.
  {{since('2024.09.02-c5476b89', inline=True)}}

* `cluster` - optional boolean. Set this to true to explicit use a cluster-enabled
  client. The default is false if the number of nodes is one, or true if the
  number of nodes is greater than one. You generally don't need to set this,
  but can do so to force a cluster connection when you have only a single
  node address. {{since('2024.09.02-c5476b89', inline=True)}}

The returned connection handle has a single `"query"` method:

## `conn:query(CMD, [ARGS])`

Issue a redis command and return the result.

See [Redis Commands](https://redis.io/commands/) for a list of commands.

The redis [INCRBY](https://redis.io/commands/incrby/) command increments a key by a value; it has the syntax:

```
INCRBY key increment
```

To use *INCRBY* to increment `my-key` by `2`:

```
conn:query("INCRBY", "my-key", 2)
```
