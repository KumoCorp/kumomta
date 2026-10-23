---
description: "Remove or hide the Received and X-KumoRef trace headers for SMTP and HTTP injection — and why you usually want to keep X-KumoRef."
---

# How Do I Remove or Hide the Received / KumoMTA / X-KumoRef Headers?

KumoMTA adds trace headers by default: a `Received:` header (which includes `KumoMTA`) and a supplemental `X-KumoRef` header. You can control both. Where you set the option depends on how mail is injected.

## SMTP injection

Set `trace_headers` on the SMTP listener:

```lua
kumo.start_esmtp_listener {
  listen = '0.0.0.0:25',
  trace_headers = {
    received_header = false, -- omit the Received: header
    supplemental_header = false, -- omit X-KumoRef
  },
}
```

!!! warning
    Listener settings are applied at startup. After changing `trace_headers` on `start_esmtp_listener` you must **restart** KumoMTA for it to take effect (it does not hot-reload).

## HTTP injection

For the injection API, set `trace_headers` in the inject request itself, not on `start_http_listener` (setting it there errors):

```json
{
  "trace_headers": {
    "received_header": false,
    "supplemental_header": false
  }
}
```

## Should you remove X-KumoRef?

Usually not. `X-KumoRef` is what correlates out-of-band bounces back to the original message, and it is very unlikely to affect inbox placement. Removing it makes bounce attribution harder. Consider keeping `X-KumoRef` even when you suppress the `Received:` header.

## See also

* [start_esmtp_listener / trace_headers](../reference/kumo/start_esmtp_listener/trace_headers.md)
* [HTTP Injection API](../reference/http/kumod/api_inject_v1_post.md)
