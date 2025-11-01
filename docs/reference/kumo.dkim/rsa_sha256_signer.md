# rsa_sha256_signer

```lua
kumo.dkim.rsa_sha256_signer { PARAMS }
```

Create a DKIM signer that uses RSA SHA256.

```lua
-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    key = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```

A signer object employs two-level caching to manage the loading of the
underlying key (`dkim_key_cache`) as well as the higher level signing object
itself (`dkim_signer_cache`).  See
[kumo.set_lruttl_cache_capacity](../kumo/set_lruttl_cache_capacity.md) for
tuning the maximum cache capacity.

`PARAMS` is a lua table that can have the following keys:

## domain

Required. The domain for which the mail is being signed.

## selector

Required. The selector used for signing

## headers

Required. The list of headers which should be signed.

## atps

Optional string. Allows setting the [Authorized Third-Party
signature](https://www.rfc-editor.org/rfc/rfc6541.html).

## atpsh

Optional string. Set the [Authorized Third-Party
Signature](https://www.rfc-editor.org/rfc/rfc6541.html) hashing algorithm.

## agent_user_identifier

Optional string. Sets the [Agent of User Identifier
(AUID)](https://www.rfc-editor.org/rfc/rfc6376.html#section-2.6) to use for
signing.

## expiration

Optional number. Sets the number of seconds from now to use for
the signature expiration.

## body_length

Optional boolean. If `true`, the body length will be included
in the signature.

## reporting

Optional boolean. If `true`, the signature will be marked as
requesting reports from the receiver/verifier.

## header_canonicalization

Specify the canonicalization method to be used when hashing message
headers.  Can be one of:

* `"Relaxed"` - this is the default
* `"Simple"`

## body_canonicalization

Specify the canonicalization method to be used when hashing message
body.  Can be one of:

* `"Relaxed"` - this is the default
* `"Simple"`

## key

Required. Specify the signing key data.

The value is a [KeySource](../keysource.md).

The key data must be either RSA PEM or a PKCS8 PEM encoded.

```lua
local file_signer = kumo.dkim.rsa_sha256_signer {
  domain = msg:from_header().domain,
  selector = 'default',
  headers = { 'From', 'To', 'Subject' },
  key = '/path/to/example-private-dkim-key.pem',
}
```

!!! tip
    The [KeySource](../keysource.md) page explains how to read from
    [HashiCorp Vault](https://www.hashicorp.com/products/vault) or from an
    arbitrary source of data.


## ttl

Optional number. Specifies the time-to-live (TTL) in KumoMTA's DKIM signer
cache.  The default is `300` seconds.

Each call to this function with the same parameters is cached for up to the
specified TTL in order to avoid the overhead of repeatedly load the key from
disk.

{{since('2025.03.19-1d3f1f67', indent=True)}}
    The ttl now supports a duration string like `"5 mins"`

## over_sign

{{since('2024.06.10-84e84b89', indent=True)}}

    Optional boolean. If `true` then the list of `headers` will be adjusted
    to match the email message being signed so that the message is signed
    in such a way that a replay attack cannot forge additional headers
    without invalidating the signature.

    The way this works is by counting the number of headers in the message,
    so if you set:

    ```lua
    headers = {'From', 'To', 'Subject'},
    ```

    and the message had 1 instance each of `From` and `To`, but was, for whatever
    reason, missing the `Subject` header, it would compute the effective header
    list as:

    ```
    headers = {'From', 'From', 'To', 'To', 'Subject'},
    ```

    In other words, it will compute `N` as the number of times each of your listed
    headers are found in the email to be signed, then treat it as though you listed
    that name `N+1` times in your configuration.


