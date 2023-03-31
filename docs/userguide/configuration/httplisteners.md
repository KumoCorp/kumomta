# Configuring HTTP Listeners

An HTTP listener can be defined with a ```kumo.start_http_listener``` function.  In the example below you can see the definition of IP address, Port, and specific trusted hosts that are permitted to to use that listener.

Each listener can have its own trust list, hostname and TLS settings.
```console
 kumo.start_http_listener {
    listen = '0.0.0.0:8000',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
    use_tls = true,
  }
  ```

  Refer to the Reference Manual for detailed options: 
  https://docs.kumomta.com/reference/kumo/start_http_listener/