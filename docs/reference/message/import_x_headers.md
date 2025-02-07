# `message:import_x_headers([NAMES])`

When called with no parameters, iterates the headers of the message, and for
each header with an `"X-"` prefix, imports the header into the message
metadata.

When called with a list of header names, only those headers, if present in the
message, will be imported to the message metadata.

When importing an `X-` header, the header name is normalized to lowercase and
`-` is transformed to underscore.

For example, with a message content of:

```
X-Campaign-ID: 12345
X-Mailer: foobar
Subject: the subject

The body
```

calling:

```lua
message:import_x_headers()
print(message:get_meta 'x_campaign_id') -- prints 12345
print(message:get_meta 'x_mailer') -- prints foobar
```

but calling:

```lua
message:import_x_headers { 'x-campaign-id' }
print(message:get_meta 'x_campaign_id') -- prints 12345
print(message:get_meta 'x_mailer') -- prints nothing
```
