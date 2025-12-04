---
tags:
  - aaa
---

# kumo.aaa.query_resource_access

{{since('dev')}}

```lua
local result = kumo.aaa.query_resource_access(RESOURCE, AUTH_INFO, PRIVILEGE)
```

!!! note
    This function is currently rather abstract and of limited utility,
    and that is reflected by the lack of examples on this page.

This function can be used to check whether an `AUTH_INFO` has `PRIVILEGE`
access to `RESOURCE`.  This is useful when performing access control checks
within your own custom policy.

`RESOURCE` is a resource object identifying the resource that is being accessed.

`PRIVILEGE` is a string describing the nature of the access privilege that is being attempted.

`AUTH_INFO` is a lua object describing the authentication status of the current session.
You will typically obtain the auth info from the connection metadata, but you can also
define an auth info for yourself in code if you have an advanced use case.

