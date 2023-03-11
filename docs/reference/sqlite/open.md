# `sqlite.open(PATH, [BUSY_TIMEOUT=500])`

Opens the sqlite database at `PATH` and returns a connection handle.

`BUSY_TIMEOUT` specifies the time in milliseconds over which sqlite
should retry operations when it is unable to get an exclusive lock.
The default is 500ms.

The returned connection object has a single method, `execute` which can
be used to execute a query.

```admonish
when using the special path `:memory:`, sqlite will create an in-memory
database which is great for this contrived example, but not a great
deal of use in a real worl usage inside KumoMTA.
```

```lua
local sqlite = require 'sqlite'
local db = sqlite.open ':memory:'

-- For the sake of this example, populate with some simple data
db:execute 'CREATE TABLE people (name, age)'

-- You can use prepared statements with positional arguments like this:
db:execute('INSERT INTO people (name, age) values (?, ?)', 'john', 42)

-- and with named arguments like this:
db:execute(
  'INSERT INTO people (name, age) values (:name, :age)',
  { name = 'fred', age = 32 }
)

-- Lets print out just the ages from the database; when the query
-- returns only a single column, the returned value will be an
-- array style lua table consisting of just the values.  In this case,
-- it is equivalent to `{32, 42}`
print 'query ages'
local ages = db:execute 'select age from people order by age'
for k, v in ipairs(ages) do
  print(k, v)
end

-- When multiple columns are returned, they are presented as
-- an array style table of lua object style tables will be returned.
-- In this case it is equivalent to:
-- `{ {name="john", age=42 }, {name="fred", age=32}}`
print 'query all'
local ages = db:execute 'select * from people order by age'
for k, v in ipairs(ages) do
  print('row', k)
  for name, value in pairs(v) do
    print(name, value)
  end
end

-- When no rows are returned by the query, the return value is
-- the number of rows affected by the query. In this case, because
-- 2 records are being deleted, this will print 2
print('deleted rows:', db:execute 'delete from people')
```

sqlite queries are executed via a thread pool so that the query won't
block important IO scheduling.

Queries and query results are not implicitly cached.
