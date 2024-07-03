# How Can I Apply Multiple DKIM Signatures to a Message?

Applying multiple DKIM signatures to a message is straightforward, but will depend on how you implement DKIM signing in your installation:

## Using the DKIM Helper

If you are using the DKIM Helper to manage the signing of messages, you can add additional DKIM signatures by using the `additional_signatures` option in your helper config, combined with the `signature` block:

{% call toml_data() %}
additional_signatures = ["MyESPName"]

[signature."MyESPName"]
# Policy is interpreted differently for these
policy = "Always" # Always add this signature
#policy = "OnlyIfMissingDomainBlock" # Use this as a fallback

# specifies the signing domain for this signature block
domain = "myesp.com"
{% endcall %}

You can add as many additional signatures as needed, and selectively decide whether those signatures are used globally or only when the sending domain does not have a signature of its own.

## Using Lua

If you are directly controlling DKIM signing using Lua, additional signatures are simply a matter of calling the signing module more than once:

```lua
kumo.on('smtp_server_message_received', function(msg)
  local signer_one
  kumo.dkim.rsa_sha256_signer {
    domain = msg:from_header().domain,
    selector = 'myselector',
    headers = { 'Content-Type', 'Message-Id', 'Subject' },
    key = '/opt/kumomta/etc/dkim/mydomain.com/myselector.key',
  }

  msg:dkim_sign(signer_one)

  local signer_two
  kumo.dkim.rsa_sha256_signer {
    domain = 'my_esp_domain.com',
    selector = 'my_esp_selector',
    headers = { 'Content-Type', 'Message-Id', 'Subject' },
    key = '/opt/kumomta/etc/dkim/my_esp_domain.com/my_esp_selector.key',
  }

  msg:dkim_sign(signer_two)
end)
```

Because you have granular control in Lua for signing, you can add as many signers as you wish, and set them either programmatically or arbitrarily depending on your needs.
