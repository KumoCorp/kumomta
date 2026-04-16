# append_part

```lua
mimepart:append_part(PART)
```

{{since('2025.10.06-5ec871ab')}}

Appends `PART`, which must also be a [MimePart](index.md) (perhaps created via
[kumo.mimpart.new_text](../kumo.mimepart/new_text.md) or similar) to the set of
child parts in `mimepart`.

This is potentially useful when constructing complex multipart messages.  You
are responsible for ensuring that the resulting mime tree makes logical sense;
`mimepart` should have a `Content-Type` that is recognized as being a multipart
container of some kind.

You might consider instead using
[kumo.mimepart.builder](../kumo.mimepart/builder.md) for a simpler message
building experience.

