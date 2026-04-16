# mailbox_list

```lua
local mailbox_list = header.mailbox_list
```

{{since('2025.10.06-5ec871ab')}}

Reading the `mailbox_list` field will attempt to interpret the contents of the
header as an [MailboxList](../headermap/index.md#mailboxlist).

If the header value is not compatible with this representation, a lua error
will be raised.
