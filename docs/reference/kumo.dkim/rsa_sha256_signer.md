# `kumo.dkim.rsa_sha256_signer {PARAMS}`

Create a DKIM signer.

```lua
-- Called once the body has been received.
-- For multi-recipient mail, this is called for each recipient.
kumo.on('smtp_server_message_received', function(msg)
  local signer = kumo.dkim.rsa_sha256_signer {
    domain = msg:sender().domain,
    selector = 'default',
    headers = { 'From', 'To', 'Subject' },
    file_name = 'example-private-dkim-key.pem',
  }
  msg:dkim_sign(signer)
end)
```
