# id

```lua
local id = mimepart.id
```

!!! note
    This is a field rather than a method, so you must use `mimepart.id`
    rather than `mimepart:id` to reference it.

{{since('2025.10.06-5ec871ab')}}

The `id` field represents the position of the part within the mime tree at the
time that it was parsed from its containing message.

This has very limited use at the current time.



