# max_smtp_trace_detail

{{since('dev')}}

Sets the maximum detail for both inbound and outbound SMTP traces served by this HTTP listener. It applies to all callers who have `GET` access to the relevant trace endpoint. If the caller does not request a lower level, the listener sends its configured maximum.

The levels, in increasing order, are `protocol`, `operational`, `envelope`, `content`, and `secrets`. The default is `secrets`: a full trace can include message bodies and SMTP AUTH credentials. See [Choosing SMTP trace detail](../../../userguide/operation/kcli.md#choosing-smtp-trace-detail) for what each level includes.

```lua
kumo.start_http_listener {
  listen = '127.0.0.1:8000',
  max_smtp_trace_detail = 'operational',
}
```
