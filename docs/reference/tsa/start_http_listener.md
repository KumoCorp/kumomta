# `kumo.start_http_listener { PARAMS }`

{{since('2023.08.22-4d895015')}}

Configure and start the TSA HTTP service.

This function should be called only from inside your
[tsa_init](../events/tsa_init.md) event handler.

This function behaves exactly like
[kumo.start_http_listener](../kumo/start_http_listener/index.md) except that it will
start the Traffic Shaping Automation specific HTTP service.
