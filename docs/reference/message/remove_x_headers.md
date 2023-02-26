# `message:remove_x_headers([NAMES])`

When called with no parameters, removes all headers with an `"X-"` prefix from
the message.

When called with a list of header names, only those headers, if present in the
message, will be removed from the message.

For example, with a message content of:

```
X-Campaign-ID: 12345
X-Mailer: foobar
Subject: the subject

The body
```

calling:

```lua
message:remove_x_headers()
```

will result in the body changing to:

```
Subject: the subject

The body
```

but calling:

```lua
message:import_x_headers { 'x-campaign-id' }
```

will result in the body changing to:

```
X-Mailer: foobar
Subject: the subject

The body
```
