---
tags:
  - aaa
---

# kumo.aaa.make_http_url_resource

{{since('dev')}}

```lua
local resource = kumo.aaa.make_http_url_resource(LOCAL_ADDR, HTTP_URL)
```

This function can be used to create a *resource* object that represents an HTTP
URL in the HTTP listener.  The two parameters are:

 * `LOCAL_ADDR` - a string like `127.0.0.1:8080` defining the local address
   (NOT the peer address!) of the HTTP listener endpoint.  This local address
   doesn't have to be a loopback address; it should map to the local "sockname"
   of the connected HTTP session and can be the external/public IP of the local
   system.
 * `HTTP_URL` - a URL string like `https://localhost/api/admin/baz` defining
   the endpoint to which access is being requested.

The eagle eyed reader will realize that `LOCAL_ADDR` is somewhat redundant with
the host portion of the URL.  They are separate parameters because the
underlying Rust logic needs to process an URL from a live HTTP request and that
may have a hostname supplied by the client.  Authorization checks are always
performed using the local address regardless of the hostname coming from the client.

The HTTP URL Resource object encodes the specific resource name that exactly
matches the combination of `LOCAL_ADDR` and `HTTP_URL`, but also encodes a list
of fall-back resource names that allow ACL rules to be effectively inherited
when performing ACL checks.

For example, when constructing a resource object like this:

```lua
local resource = kumo.aaa.make_http_url_resource(
  '127.0.0.1:8080',
  'https://localhost/foo/bar/baz'
)
```

produces a resource object that, when used as part of an ACL query via
[kumo.aaa.query_resource_access](query_resource_access.md), will cause the
following sequence of resource names to be loaded via
[get_acl_definition](../events/get_acl_definition.md) until that event returns
an ACL:

 * `http_listener/127.0.0.1:8080/foo/bar/baz`
 * `http_listener/127.0.0.1:8080/foo/bar`
 * `http_listener/127.0.0.1:8080/foo`
 * `http_listener/127.0.0.1:8080`
 * `http_listener/*/foo/bar/baz`
 * `http_listener/*/foo/bar`
 * `http_listener/*/foo`
 * `http_listener`

You can see that this sequence of resource names corresponds to a tree-like
structure that allows you to, for example, set rules on a very specific
resource path such as `http_listener/127.0.0.1:8080/foo/bar/baz` that either
widen or narrow the scope of a more general rule that might be defined on a
path such as `http_listener/*/foo/bar`.

The product default ACLs take advantage of this structure to define a general
access rule for `http_listener/*/api/admin`.



