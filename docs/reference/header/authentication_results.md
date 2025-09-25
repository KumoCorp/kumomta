# authentication_results

```lua
local authentication_results = header.authentication_results
```

{{since('dev')}}

Reading the `authentication_results` field will attempt to interpret the contents of the
header as an [Authentication Result](../authenticationresult.md).

If the header value is not compatible with this representation, a lua error
will be raised.
