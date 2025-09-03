# The HeaderMap Object

{{since('dev')}}

Represents the parsed state of the set of headers in a [MimePart](../mimepart/index.md).

The headermap provides access to the headers and allows modification of the set of headers.

!!! note
    While the map allows modification of the set of headers, the individual
    header objects that it returns are copies of the parsed headers; if you
    wish to modify the headers, you must explicitly use the methods of the
    header map to apply those changes to the headermap.

!!! info
    Printing or otherwise explicitly converting a `HeaderMap` object as a string
    will produce the RFC 5322 representation of the headers contained in that map.

## Associated Data Types

The following data types/representations are associated with both the
`HeaderMap` and `Header` objects that can be obtained through it.  `HeaderMap`
provides accessors for fields by name/type/function which return the following
data types from the getter functions (eg: [headermap:to](to.md)) and accept
them as parameters in the setter functions (eg: [headermap:set_to](set_to.md)).

### Address

Represents an email address, which can be either a [Mailbox](index.md#mailbox)
or a [Group](index.md#group), both shown below.

```lua
-- This is an example of a `Mailbox`, which is valid as an `Address`
local address = {
  name = 'John Smith',
  address = {
    local_part = 'john.smith',
    domain = 'example.com',
  },
}
```

### AddressList

Represents a list of `Address`es (either `Mailbox` or `Group`); it is
mapped to lua as an array style table listing out the addresses.  A list can
have 0 or more entries.

```lua
local addresses = {
  -- The first entry is a mailbox
  {
    name = 'John Smith',
    address = {
      local_part = 'john.smith',
      domain = 'example.com',
    },
  },

  -- The second entry is also a mailbox
  {
    name = 'Joe Bloggs',
    address = {
      local_part = 'joe.bloggs',
      domain = 'example.com',
    },
  },

  -- The third entry is a group
  {
    name = 'The A Team',
    entries = {
      {
        name = 'Bodie',
        address = {
          local_part = 'bodie',
          domain = 'example.com',
        },
      },
      {
        address = {
          local_part = 'doyle',
          domain = 'example.com',
        },
      },
      {
        address = {
          local_part = 'tiger',
          domain = 'example.com',
        },
      },
      {
        address = {
          local_part = 'the.jewellery.man',
          domain = 'example.com',
        },
      },
    },
  },
}
```

### Group

Represents the group addressing syntax; groups are typically shown, by default,
in the MUA collapsed down to just the `name` portion, making the overall
distribution list less overwhelming in its default presentation.

```lua
-- This is an example of a `Group`, which is valid as an `Address`
local group = {
  name = 'The A Team', -- the display name for the group
  entries = { -- `entries`, rather than `address` is what distinguishes this from a mailbox
    {
      name = 'Bodie',
      address = {
        local_part = 'bodie',
        domain = 'example.com',
      },
    },
    {
      address = {
        local_part = 'doyle',
        domain = 'example.com',
      },
    },
    {
      address = {
        local_part = 'tiger',
        domain = 'example.com',
      },
    },
    {
      address = {
        local_part = 'the.jewellery.man',
        domain = 'example.com',
      },
    },
  },
}
```

### Mailbox

Represents an individual mailbox (email address)

```lua
-- This is an example of a `Mailbox`
local mailbox = {
  name = 'John Smith', -- an optional string holding the display name
  address = {
    local_part = 'john.smith',
    domain = 'example.com',
  },
}
```

### MailboxList

Represents a list of `Mailbox`es; it is mapped to lua as an array style table
listing out the mailboxes.  A list can have 0 or more entries.

```lua
local mailboxes = {
  -- The first entry
  {
    name = 'John Smith',
    address = {
      local_part = 'john.smith',
      domain = 'example.com',
    },
  },

  -- The second entry
  {
    name = 'Joe Bloggs',
    address = {
      local_part = 'joe.bloggs',
      domain = 'example.com',
    },
  },
}
```

## Available Fields and Methods { data-search-exclude }
