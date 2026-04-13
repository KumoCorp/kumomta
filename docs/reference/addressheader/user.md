# user

```lua
local user = addressheader.user
```

If the address header consists of a single simple address, returns the mailbox
portion of the address. For example, if the address is
`"first.last@example.com"`, `addressheader.domain` will evaluate as
`"first.last"`.

If the address header is not a single simple address, raises an error.

See also [addressheader.domain](domain.md), [addressheader.name](name.md).

{{since('dev')}}

The `user` field now returns the *normalized/decoded* local part.  Previously,
quoted local parts such as `"quoted"@example.com` were returned with the
RFC 5322 quoting intact (e.g. `"quoted"`); they now return the decoded
semantic value (e.g. `quoted`).  The same applies to entries in
[addressheader.list](list.md).


