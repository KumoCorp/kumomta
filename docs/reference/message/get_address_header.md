# `message:get_address_header(NAME)`

Gets the first header whose name matches `NAME`, parses it as a list of mailboxes and groups, and returns an [addressheader](../addressheader/index.md) object.

Returns `nil` if no matching header could be found.

See also [message:from_header](from_header.md) and [message:to_header](to_header.md).

