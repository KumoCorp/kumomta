# connection_limit

Specifies the maximum number of concurrent connections that will be made from
the current MTA machine to the destination site.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = 32,
  }
end)
```

{{since('dev')}}

`connection_limit` may now be specified using a string in addition to an integer;
the string format allows the use of commas or underscores to separate digits
if you prefer that for clarity:

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = '1,000',
  }
end)
```

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = '1_000',
  }
end)
```

However, the primary reason for supporting the string notation is that
you can indicate when you wish for the connection limit to be local
to the kumod instance, rather than shared via redis.

In the example below, we force the use of a local connection limit even if
redis-shared throttle and limits are enabled:


```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    connection_limit = 'local:20',
  }
end)
```
