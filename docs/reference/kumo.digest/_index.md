# Module `kumo.digest`

This module provides functions for hashing/digesting data.

## The Digest Object

The functions in this module return a `Digest` object.

Printing or otherwise explicitly converting a `Digest` object
as a string will produce the digest bytes encoded in hex.

The following fields are available to return the bytes encoded
in various ways.

* `bytes` - returns the data as a binary byte string. This is the most compact representation, but is difficult to pass into other systems without encoding in some way. Case sensitive.
* `hex` - returns the data encoded as lowercase hexadecimal. This is the largest representation. Case insensitive.
* `base32` - returns the data encoded as base32. Case insensitive.
* `base32_nopad` - same as `base32`, but does not include padding characters.
* `base32hex` - returns the data encoded as base32hex. This is similar to `base32`, but the encoded version preserve the sort order of the input data. Case insensitive.
* `base32hex_nopad` - same as `base32hex`, but does not include padding characters.
* `base64` - returns the data encoded as base64. Case sensitive.
* `base64_nopad` - same as `base64`, but does not include padding characters.
* `base64url` - returns the data encoded as base64, with a URL-safe alphabet. Case sensitive.
* `base64url_nopad` - same as `base64url`, but does not include padding characters.

```lua
-- Compute the digest of 'hello'
local d = kumo.digest.sha1 'hello'
-- Demonstrate the various output properties of the digest object
assert(tostring(d) == d.hex)
assert(d.hex == 'aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d')
assert(
  d.bytes
    == '\xaa\xf4\xc6\x1d\xdc\xc5\xe8\xa2\xda\xbe\xde\x0f\x3b\x48\x2c\xd9\xae\xa9\x43\x4d'
)
assert(d.base32 == 'VL2MMHO4YXUKFWV63YHTWSBM3GXKSQ2N')
assert(d.base32_nopad == 'VL2MMHO4YXUKFWV63YHTWSBM3GXKSQ2N')
assert(d.base32hex == 'LBQCC7ESONKA5MLURO7JMI1CR6NAIGQD')
assert(d.base64 == 'qvTGHdzF6KLavt4PO0gs2a6pQ00=')
assert(d.base64_nopad == 'qvTGHdzF6KLavt4PO0gs2a6pQ00')
assert(d.base64url == 'qvTGHdzF6KLavt4PO0gs2a6pQ00=')
assert(d.base64url_nopad == 'qvTGHdzF6KLavt4PO0gs2a6pQ00')
```

## Available Functions { data-search-exclude }
