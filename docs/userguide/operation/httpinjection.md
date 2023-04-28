# Injecting Using HTTP

KumoMTA will listen for message injection in any [listener](https://docs.kumomta.com/reference/kumo/start_esmtp_listener/) [defined](https://docs.kumomta.com/reference/kumo/start_http_listener/) in configuration. You have complete control over the IPs and Ports available for message injection.

The HTTP Listener will accept any properly formatted HTTP connection request allowed by its configuration.  For instance, based on this:
```console
kumo.start_http_listener {
    use_tls = true,
    listen = '0.0.0.0:8005',
    -- allowed to access any http endpoint without additional auth
    trusted_hosts = { '127.0.0.1', '::1' },
  }
```
KumoMTA will accept any HTTPS injection on port 8005 from the local host ONLY. (This also enables the full [HTTP API](https://docs.kumomta.com/reference/http/) from localhost).


The simplest test of [HTTP injection](https://docs.kumomta.com/reference/http/api_inject_v1/) can be done using cURL right from localhost console.

```console
curl -i 'http://localhost:8005/api/inject/v1' 
 -H 'Content-Type: application/json' -d '
{"envelope_sender": "noreply@example.com",
 "content": "Subject: hello\n\nHello there",
 "recipients": [{"email": "recipient@example.com"}]
}'
```
That should return something like this:
`{"success_count":1,"fail_count":0,"failed_recipients":[],"errors":[]}`

Any system that can use an HTTP API to pass JSON should work as an injection system if you follow the JSON payload formatting rules posted [here](https://docs.kumomta.com/reference/http/api_inject_v1/) 



