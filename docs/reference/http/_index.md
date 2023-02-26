# HTTP API

HTTP service is configured via [kumo.start_http_listener](../kumo/start_http_listener.md).

## Authentication

All HTTP endpoints require that the client satisfy one of the follow conditions:

* Trusted IP - Connecting from a host covered by the
  [trusted_hosts](../kumo/start_http_listener.md#trusted_hosts) defined for the
  HTTP listener
* Authenticated - Provide HTTP Basic authentication credentials that are
  validated successfully by the
  [http_server_validate_auth_basic](../events/http_server_validate_auth_basic.md)
  event handler

## Endpoints

The following endpoints are available:


