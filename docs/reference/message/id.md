# id

```lua
message:id()
```

Each message is uniquely identified by the system using an id.

At the time of writing, the id is a version 1
[UUID](https://www.rfc-editor.org/rfc/rfc4122), which is based upon the time at
which the message was created by the system. In the future, the system may
change to adopt the newer v7 UUID.

`message:id()` returns the UUID encoded as a hexadecimal string, without any
dashes or braces.


