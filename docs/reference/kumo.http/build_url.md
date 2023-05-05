# `kumo.http.build_url(base, {PARAMS})`

Given a base URL and a set of parameters, combine the two together
and return the result as a string.

`PARAMS` is an object-style table consisting of key/value pairs that
should be added as GET parameters to the URL:

```lua
local url = kumo.http.build_url('https://example.com/?existing=value', {
  a = 1,
  b = 2,
})
assert(url == 'https://example.com/?existing=value&a=1&b=2')
```

