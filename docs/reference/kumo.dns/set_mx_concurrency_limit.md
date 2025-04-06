# `kumo.dns.set_mx_concurrency_limit(LIMIT)`

{{since('dev')}}

Set the maximum number of concurrent MX lookups that we will allow
to send to the upstream DNS resolver.

The default is 128.

In earlier versions of kumomta, there was no default, issuing
as many queries as we were requested to make at any given moment.

```lua
kumo.on('pre_init', function()
  kumo.dns.set_mx_concurrency_limit(128)
end)
```


