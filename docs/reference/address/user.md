# user

```lua
local user = address.user
```

Returns the mailbox portion of the address. For example, if the address is
`"first.last@example.com"`, `address.user` will evaluate as `"first.last"`.

See also [address.domain](domain.md).

## Quoted Local Part

{{since('2026.04.09-ea3b2a9b')}}

Prior releases of KumoMTA had inconsistent behavior around handling envelope
addresses whose local part was a quoted string.  The behavior has been improved,
and this `user` field will now return the *normalized local part* from the address.

Normalization removes any quoting and quotes from the local part.

Some examples of how this works are shown in the table below:

|Address|Normalized Local Part|
|-------|---------------------|
|`foo@example.com`|`foo`|
|`"foo"@example.com`|`foo`|
|`"f\oo"@example.com`|`foo`|
|`"info@"@example.com`|`info@`|



