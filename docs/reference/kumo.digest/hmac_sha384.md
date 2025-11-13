# hmac_sha384

```lua
kumo.digest.hmac_sha384(KEY, MSG)
```

{{since('dev')}}

Computes an [HMAC](https://tools.ietf.org/html/rfc2104) digest over the `MSG`
argument, using `KEY` as the means for signing/authenticating the data.

This function uses `SHA384` as the digest algorithm for the HMAC.

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
local hmac = kumo.digest.hmac_sha384(key_bytes, 'your message')
assert(
  hmac.hex
    == 'cd274957b95ce192d41dd52f83fd2eb9277aa2fa210ec798ee16e978801a89b7e7b956af3976d1a50a60ec|'
)
```



