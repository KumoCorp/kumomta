---
tags:
  - audit_series
---

# audit_series.define

```lua
kumo.audit_series.define(name, options)
```

{{since('dev')}}

!!!note
    audit_series will store up to 64 definitions and 256 windows. Exceeding the limit would result in evicting the least recently used cache from memory. If data persistence is a requirement, please consider using an external data store.

Register the audit series definition. Counter would be maintained using sliding window mechanism. 

`window_count` : number of sliding windows to maintain. 
`ttl` : length of given window in time duration format

Call this function from init event, repeated registration of the same name would result in an error.

```lua
  kumo.audit_series.define('failure_count', {
    window_count = 3,
    ttl = '300 second',
  })
```
