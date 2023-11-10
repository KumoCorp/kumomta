# `kumo.memoize(FUNCTION, {PARAMS})`

This function allows you to create functions that cache the result of a
computation.  This technique is known as
[Memoization](https://en.wikipedia.org/wiki/Memoization).

Here's a simple example; we have a sqlite lookup database that we intend to use
for authentication, and we have already defined a helper function for that:

```lua
local sqlite = require 'sqlite'

-- Consult a hypothetical sqlite database that has an auth table
-- with user and pass fields
function sqlite_auth_check(user, password)
  local db = sqlite.open '/path/to/auth.db'
  local result = db:execute(
    'select user from auth where user=? and pass=?',
    user,
    password
  )
  -- if we return the username, it is because the password matched
  return result[1] == user
end

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  return sqlite_auth_check(authc, password)
end)
```

Let's say, for the sake of example, that the query is very expensive: perhaps
the IO cost is high and the query rate is also high and we want to save the iops
for other work in the system.

It would be nice if we could cache the lookup for some period of time.
We can use `kumo.memoize` for this:

```lua
local sqlite = require 'sqlite'

function sqlite_auth_check(user, password)
  local db = sqlite.open '/tmp/auth.db'
  local result = db:execute(
    'select user from auth where user=? and pass=?',
    user,
    password
  )
  -- if we return the username, it is because the password matched
  return result[1] == user
end

-- This creates a new function called `cached_sqlite_auth_check`
-- that remembers the results for a given set of parameters for up
-- to 5 minutes or up to 100 different sets of parameters
cached_sqlite_auth_check = kumo.memoize(sqlite_auth_check, {
  name = 'sqlite_auth',
  ttl = '5 minutes',
  capacity = 100,
})

kumo.on('smtp_server_auth_plain', function(authz, authc, password)
  return cached_sqlite_auth_check(authc, password)
end)
```

`kumo.memoize` takes a function or a lambda and wraps it up with some logic
that will internally cache the result for the same set of parameters, and
returns a new function that encodes that caching logic.  The return value is
the *memoized function*.

The parameters it accepts are:

* *FUNCTION* - the function or lambda which will be called when there is a cache miss.
  When it is called, it will be passed the parameters that were passed to the *memoized function*.
* *PARAMS* is a required lua table with the following fields, all of which are required:
     * `name` - the name for the cache. You should create one name per function/purpose.
     * `ttl` - the Time To Live for cache entries; how long a previously computed
       value should remain valid.  The duration is expressed as a string like `5
       minutes` or `10 seconds`.
     * `capacity` - the total number of results to retain in the cache. When a new
       entry needs to be inserted, if the cache is at capacity, the eldest entry
       will be evicted to make space.

In the example above calling:

```lua
cached_sqlite_auth_check('scott', 'tiger')
```

the first time would be a cache miss, because no calls have yet been made with `{'scott', 'tiger'}` as parameters, so the memoized function would internally call:

```lua
sqlite_auth_check('scott', 'tiger')
```

The next time that `cached_sqlite_auth_check('scott', 'tiger')` is called there
is a cache hit and the previously computed result would be returned, provided that
5 minutes have not expired since the first call.

When the value expires, another call to `sqlite_auth_check('scott', 'tiger')` will
be made to determine the value.

