# The MimePart Object

A `MimePart` object represents the parsed MIME structure of an RFC 5322
formatted message.  Complex messages form a tree that is composed of various
`MimeParts` and there are multiple different ways to build a MIME tree to, for
example, represent a message with a single attachment.

`MimePart` is the name of the underlying Rust type, but it is exposed through
to lua via a `PartRef` that acts as a handle to the `MimePart` that allows
safely modifying the MIME structure.

There are few different ways to obtain a `MimePart`:

  * By parsing it from a `Message` object via [msg:parse_mime](../message/parse_mime.md)
  * By creating parts using the [kumo.mimepart](../kumo.mimepart/index.md) module

!!! info
    Printing or otherwise explicitly converting a `MimePart` object as a string
    will produce the RFC 5322 representation of that MimePart and its children.

## Available Fields and Methods { data-search-exclude }
