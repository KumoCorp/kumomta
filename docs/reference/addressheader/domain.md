# domain

```lua
local domain = addressheader.domain
```

If the address header consists of a single simple address, returns the domain
portion of the address. For example, if the address is
`"first.last@example.com"`, `addressheader.domain` will evaluate as
`"example.com"`.

If the address header is not a single simple address, raises an error.

See also [addressheader.user](user.md), [addressheader.name](name.md).

