---
tags:
  - aaa
---

# kumo.aaa.make_access_control_list

{{since('dev')}}

```lua
local acl = kumo.aaa.make_access_control_list(RULES)
```

This function creates an access control list (ACL) and returns it.

An access control list is comprised of a list of access control rules.

In the example below, an ACL is created that comprises of a single rule that
allows `GET` access to the protected resource for members of a group named
`kumomta:http-listener-trusted-ip`:

```lua
local acl = kumo.aaa.make_access_control_list {
  {
    criteria = {
      Identity = { Group = 'kumomta:http-listener-trusted-ip' },
    },
    privilege = 'GET',
    access = 'Allow',
  },
}
```

This function is generally only useful within the context of the
[get_acl_definition](../events/get_acl_definition.md) event callback, where you might use it something like this:

```lua
kumo.on('get_acl_definition', function(resource)
  if resource == 'http_listener/*/api/admin' then
    return kumo.aaa.make_access_control_list {
      {
        criteria = {
          Identity = { Group = 'kumomta:http-listener-trusted-ip' },
        },
        privilege = 'GET',
        access = 'Allow',
      },
    }
  end
  -- Unknown resource, don't return an ACL
  return nil
end)
```

!!! note
    It is generally much more convenient to define ACLs using the TOML
    file syntax described in [kumo.aaa.load_acl_map](load_acl_map.md),
    so it is recommended to use that approach when possible.

The `RULES` parameter to `kumo.aaa.make_access_control_list` is an array-style
holding one rule per entry.

Each rule has the following fields:

 * `criteria` - the condition that must evaluated to true in order for this
   particular rule to match and apply.  See below for more details on this.
 * `privilege` - the name of the privilege that is being requested.  The
   privilege and criteria must match in order for the rule to apply to
   the current ACL query.  A `privilege` is a string that defines some action
   on a particular resource.  For example, the HTTP listener will take the
   HTTP request method and use that as the privilege string when deciding
   when a given request is authorized.
 * `access` - the access level that is applied when both criteria and privilege
   match.  This is either the string `Allow` to indicate that access is granted,
   or `Deny` to indicate that access is denied.

The criteria allows matching authenticated identities, group membership, peer
ip addresses and simple boolean operations that allow grouping together
multiple criteria.

The Identity primitives are:

## Individual Identity

```lua
criteria = {
  Identity = { Individual = 'john.smith' },
}
```

This condition is satisfied (evaluates as `true`) when the auth into lists
the specified user name in its list of authenticated identities.

## Group Membership

```lua
criteria = {
  Identity = { Group = 'kumomta:http-listener-trusted-ip' },
}
```

This condition is satisfied (evaluates as `true`) when the auth info lists the
specified group name in its list of groups.

In the example above the `kumomta:http-listener-trusted-ip` group is
automatically added to the group membership when the peer-ip associated with
the incoming IP address is among the set of
[trusted_hosts](../kumo/start_http_listener/trusted_hosts.md) defined in the
HTTP listener.

Other groups may be populated into the auth info through authentication related
event callbacks, depending on the context.

## Any Authenticated Identity

```lua
criteria = {
  Identity = { Authenticated = {} },
}
```

This condition is satisfied (evaluates as `true`) when the auth info lists at
least one user identity in its list of identities, which is the case when
authentication has been performed successfully.

## Unauthenticated Identity

```lua
criteria = {
  Identity = { Unauthenticated = {} },
}
```

This condition is satisfied (evaluates as `true`) when the auth info has no
authenticated identities associated with it, which is true when no
authentication has been performed, or has not been performed successfully.

## Matching any authentication state

```lua
criteria = {
  Identity = { Any = {} },
}
```

This condition is always satisfied (evaluates as `true`) regardless of the
authentication state, identity or group membership of the current session.

## Grouping: AllOf

The `AllOf` condition is satisifed (evaluates as `true`) when all of the
conditions defined within it also evaluate to true.  This is a logical `AND`
operation.

In the example below, the critiera will only evaluate as true if the session
belongs to a hypothetical `admins` group AND if the peer address is `10.0.0.1`:

```lua
criteria = {
  AllOf = {
    { Identity = { Group = 'admins' } },
    { Machine = '10.0.0.1' },
  },
}
```

## Grouping: AnyOf

The `AnyOf` condition is satisifed (evaluates as `true`) when any of the
conditions defined within it also evaluate to true.  This is a logical `OR`
operation.

In the example below, the critiera will evaluate as true if either the session
belongs to a hypothetical `admins` group OR if the peer address is `10.0.0.1`:

```lua
criteria = {
  AnyOf = {
    { Identity = { Group = 'admins' } },
    { Machine = '10.0.0.1' },
  },
}
```

## Inverting Criteria

The `Not` condition is satisifed (evaluates as `true`) when the term within it
evaluates to `false`.  This is a logical `NOT` operation.

In the example below, the critiera will evaluate as true if the session is NOT
a member of a hypothetical `admins` group.

```lua
criteria = {
  Not = { Identity = { Group = 'admins' } },
}
```
