---
tags:
  - audit_series
---

# audit_series.define

```lua
kumo.audit_series.define(name, options)
```

{{since('dev')}}

Register the audit series definition. Counter would be maintained using sliding window mechanism. 

`bucket_count` : number of sliding buckets to maintain. 

`window` : length of given window in time duration format

Call this function from init event, repeated registration of the same name would result in an error.

```
  kumo.audit_series.define('failure_count', {
    bucket_count = 3,
    window = '300 second',
  })
```
