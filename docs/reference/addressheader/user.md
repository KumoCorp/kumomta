# `addressheader.user`

If the address header consists of a single simple address, returns the mailbox
portion of the address. For example, if the address is
`"first.last@example.com"`, `addressheader.domain` will evaluate as
`"first.last"`.

If the address header is not a single simple address, raises an error.

See also [addressheader.domain](domain.md), [addressheader.name](name.md).


