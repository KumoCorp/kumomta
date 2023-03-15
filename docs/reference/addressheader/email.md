# `addressheader.email`

If the address header consists of a single simple address, returns the email
address portion of the address. For example, if the address is `"John Smith
<first.last@example.com>`, `addressheader.email` will evaluate as
`first.last@example.com`.

If the address header is not a single simple address, raises an error.

If the address header is a single simple address, but has no email address,
returns `nil`.

