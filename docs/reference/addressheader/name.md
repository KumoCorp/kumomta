# `addressheader.name`

If the address header consists of a single simple address, returns the display name
portion of the address. For example, if the address is
`"John Smith <first.last@example.com>`, `addressheader.name` will evaluate as
`"John Smith"`.

If the address header is not a single simple address, raises an error.

If the address header is a single simple address, but has no display name,
returns `nil`.

See also [addressheader.user](user.md), [addressheader.domain](domain.md).


