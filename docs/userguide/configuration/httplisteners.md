# Configuring HTTP Listeners

An HTTP listener can be defined with a `kumo.start_http_listener` function.  In the example below you can see the definition of IP address, Port, and specific trusted hosts that are permitted to to use that listener.

Each listener can have its own trust list, hostname and TLS settings.

```lua
kumo.start_http_listener {
  listen = '0.0.0.0:8000',
  -- allowed to access any http endpoint without additional auth
  trusted_hosts = { '127.0.0.1', '::1' },
  use_tls = true,
}
```

  Refer to the [Reference Manual](https://docs.kumomta.com/reference/kumo/start_http_listener/) for detailed options.

  ## What can you use the HTTP listener for?
  Aside from injecting messages using the [Inject API](https://docs.kumomta.com/reference/http/api_inject_v1/), you can also perform arbitrary administrative bounces, and collect detailed metrics.  A list of HTTP API functions exists [here](https://docs.kumomta.com/reference/http/).

## Configuring for HTTPS
The HTTP listener can easily be secured with TLS by adding the TLS directives and a certificate to the configuration.  Below is an example of an HTTPS configuration.
```lua
kumo.start_http_listener {
  trusted_hosts = { '127.0.0.1', '::1' },
  listen = '0.0.0.0:443',
  hostname = 'mail.example.com',
  use_tls = true,
  tls_certificate = '/path/to/cert.pem',
  tls_private_key = '/path/to/key.pem',

--[[ ALternately configure to pull the certificate from HashiCorp Vault ]]--
--[[
   tls_certificate = {
    vault_mount = 'secret',
    vault_path = 'tls/mail.example.com.cert',
    vault_address = "http://127.0.0.1:8200",
    vault_token = "hvs.TOKENTOKENTOKEN",
  },
]]--

}
```