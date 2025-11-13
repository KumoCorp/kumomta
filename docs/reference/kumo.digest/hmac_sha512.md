# hmac_sha512

```lua
kumo.digest.hmac_sha512(KEY, MSG)
```

{{since('dev')}}

Computes an [HMAC](https://tools.ietf.org/html/rfc2104) digest over the `MSG`
argument, using `KEY` as the means for signing/authenticating the data.

This function uses `SHA512` as the digest algorithm for the HMAC.

The `KEY` parameter is expected to be a [KeySource](../keysource.md) defining
how to access the secret key bytes.

The returned value is a [BinaryResult](index.md) object representing the digest
bytes. It has properties that can return the digest bytes or encoded in
a number of common and useful encodings.

## Computing the HMAC with a string-based key

```lua
local kumo = require 'kumo'

-- It is not recommended to use this form in production.
-- Consider storing the key in either a vault or in a local
-- file to keep the credential material outside of the repo
-- where you maintain your lua policy code
local key_bytes = {
  key_data = 'your key',
}
local hmac = kumo.digest.hmac_sha512(key_bytes, 'your message')
assert(
  hmac.hex
    == '2f5ddcdbd062a5392f07b0cd0262bf52c21bfb3db513296240cca8d5accc09d18d96be0a94995be4494c032f1eda946ad549fb61ccbe985d160f0b2f9588d34b'
)
```


