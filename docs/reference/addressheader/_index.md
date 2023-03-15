# AddressHeader object

Represents the parsed form of an email header that holds email addresses,
such as `"From"` and `"To"` headers.

As these headers can contain lists of addresses and groups of addresses, care
needs to be taken when processing them.

Convenience accessors for the common case of a single address are provided, but
they will raise an error when used on an address that is not a simple single
address.

The [addressheader.list](list.md) field can be used to safely operate on the
parsed out set of addresses, regardless of how many are present.

Note that you can use `tostring(address)` to get a JSON rendition of the parsed
address information.

## Available Fields

