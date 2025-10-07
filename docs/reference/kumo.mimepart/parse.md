# parse

```lua
kumo.mimepart.parse 'Subject: hello\r\n\r\nbody\r\n'
```

{{since('2025.10.06-5ec871ab')}}

Accepts a single string argument and parses it into [MimePart](../mimepart/index.md) object.
The input string is expected to be an RFC 5322 formatted message.
