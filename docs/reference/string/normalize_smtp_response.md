# kumo.string.normalize_smtp_response

```lua
kumo.string.normalize_smtp_response(TEXT)
```

{{since('dev')}}

The purpose of this function is to normalize out per-user, per-transaction
variations from an input SMTP response line.  The normalization is intended to
make it easier for downstream log processing to group similar events together.

!!! note
    The normalization process is not guaranteed to remove any/all user- or
    transaction-centric content from a string, but rather to make a best effort
    attempt to tokenize out the more obvious parts of the response as a
    convenience.

    If you need stronger guarantees that PII will be removed from logs
    then you will need to make alternative arrangements.

Given an input string like:

```
550 5.1.1 The email account that you tried to reach does not exist. Please try double-checking the recipient's email address for typos or unnecessary spaces. For more information, go to  https://support.google.com/mail/?p=NoSuchUser 41be03b00d2f7-b93bf44f0c0si6882731a12.803 - gsmtp
```

The normalized version would look like:

```
550 5.1.1 The email account that you tried to reach does not exist. Please try double-checking the recipient's email address for typos or unnecessary spaces. For more information, go to https://support.google.com/mail/?p=NoSuchUser {hash} - gsmtp
```

Normalization works approximately as follows:

 * The input is tokenized into whitespace separated words
 * Dictionary words are left unchanged
 * RFC3339 timestamps are recognized and replaced by the token `{timestamp}`
 * UUIDs in a variety of formats are recognized and replaced by the token `{uuid}`
 * Alphanumeric sequences of a certain minimum length and minimum number of both
   letters AND digits with optional `.`, `-` and `_` delimiters are replaced
   by the token `{hash}`.
 * IPv4 and IPv6 addresses are replaced by the token `{ipaddr}`
 * Sequences that appear to be valid base64 or base64 URL are replaced by the
   token `{base64}`.
 * RFC5321 email addresses are replaced by the token `{email}`
 * Tokens that look like `lhs=rhs` are split on the `=` sign and the right hand
   side is recursively processed by the normalizer
