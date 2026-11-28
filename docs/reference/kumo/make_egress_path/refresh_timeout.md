# refresh_timeout

{{since('dev')}}

Optional duration string that specifies the maximum amount of
time to spend computing an updated configuration for a ready queue.

When the timeout is exceeded, an error is logged and the existing
egress path configuration is retained.

When omitted, no additional configuration refresh timeout is
imposed.