---
tags:
  - aaa
---

# kumo.aaa.load_acl_map

{{since('dev')}}

```lua
local map = kumo.aaa.load_acl_map(KEYSOURCE)
```

This function loads an ACL map definition file (whose syntax is described
below) and parses it into an ACL Map object which can be queried to provide an
implementation for the [get_acl_definition](../events/get_acl_definition.md)
event callback.

The parameter is a [keysource](../keysource.md) object which enables the ACL
definition to be loaded from a file, from an inline string, a hashicorp vault
or through a data loading event callback.

You can load from a file like so:

```lua
local map = kumo.aaa.load_acl_map '/opt/kumomta/etc/custom_acl.toml'
```

Or from an inline string:

```lua
local map = kumo.aaa.load_acl_map {
  key_data = [=[
[[acl."http_listener/*/api/admin/baz"]]
allow = true
privileges = ["GET"]
identity.Group = "kumomta:http-listener-trusted-ip"
    ]=],
}
```

or any of the other forms allowed by a [keysource](../keysource.md).

## ACL Map File Syntax

The purpose of an ACL Map File is to map a given *resource* name to an *access
control list* that defines an ordered sequence of rules that might apply to that resource.
The first rule that matches defines the access level that is allowed for the session.

The following excerpt adds a new ACL rule to the resource named `http_listener/*/api/check-liveness`.
That rule allows any access, either Authenticated or Unauthenticated sessions, to perform `GET`
requests to URLs with a path of `/api/check-liveness`.  Note the use of `[[` and `]]` around the
rule definition--that is TOML syntax for appending a new array entry to the definition, which has
the effect of adding a new rule to the ACL:

```toml
# Explicitly allow blanket unauthenticated access to the health status endpoint
[[acl."http_listener/*/api/check-liveness"]]
allow = true
privileges = ["GET"]
identity.Any = {}
```

This example shows how two rules are added to the same entry; the first one
grants `POST` access to members of the `kumomta:http-listener-trusted-ip` group
(which matches peer addresses that match the
[trusted_hosts](../kumo/start_http_listener/trusted_hosts.md) config option),
while the second grants `POST` access to any authenticated HTTP client:

```toml
# Trusted ips can use the injection API
[[acl."http_listener/*/api/inject"]]
allow = true
privileges = ["POST"]
identity.Group = "kumomta:http-listener-trusted-ip"

# Allow injection by other authenticated users
[[acl."http_listener/*/api/inject"]]
allow = true
privileges = ["POST"]
identity.Authenticated = {}
```

Each ACL rule comprises of the following fields:

 * `allow` - a boolean indicating whether this rule allows (`true`) or denies
   (`false`) access when the other criteria in the rule match.

 * `privileges` - an array listing the set of privilege names which are covered
   by this rule.  For example `["POST", "GET"]` means that an auth check for
   a privilege of either `POST` OR `GET` will potentially match this rule,
   assuming that the other criteria are a match.

 * `identity` - defines the identity matching criteria, for example, a required
   group or authentication identity.  This field is mutually exclusive with
   `criteria`, described in the next bullet.

 * `criteria` - an extended set of matching criteria that allows using logical
   operators to group or invert other identity criteria.  This is useful to
   define a rule that allows certain identities access but only if they
   are connected from an approved IP address.  This field is mutually exclusive
   with the `identity` field described in the bullet above this one.

`identity` and `criteria` examples are found below:

## Individual Identity

```toml
identity.Individual = "john.smith"
```

This condition is satisfied (evaluates as `true`) when the auth into lists
the specified user name in its list of authenticated identities.

## Group Membership

```toml
identity.Group = 'kumomta:http-listener-trusted-ip'
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

```toml
identity.Authenticated = {}
```

This condition is satisfied (evaluates as `true`) when the auth info lists at
least one user identity in its list of identities, which is the case when
authentication has been performed successfully.

## Unauthenticated Identity

```toml
identity.Unauthenticated = {}
```

This condition is satisfied (evaluates as `true`) when the auth info has no
authenticated identities associated with it, which is true when no
authentication has been performed, or has not been performed successfully.

## Matching any authentication state

```toml
identity.Any = {}
```

## Grouping: AllOf

The `AllOf` condition is satisifed (evaluates as `true`) when all of the
conditions defined within it also evaluate to true.  This is a logical `AND`
operation.

In the example below, the critiera will only evaluate as true if the session
belongs to a hypothetical `admins` group AND if the peer address is `10.0.0.1`:

```toml
criteria.AllOf = [ {Identity = { Group = 'admins' }}, {Identity={Machine = "10.0.0.1"}} ]
```

## Grouping: AnyOf

The `AnyOf` condition is satisifed (evaluates as `true`) when any of the
conditions defined within it also evaluate to true.  This is a logical `OR`
operation.

In the example below, the critiera will evaluate as true if either the session
belongs to a hypothetical `admins` group OR if the peer address is `10.0.0.1`:

```toml
criteria.AnyOf = [ {Identity = { Group = 'admins' }}, {Identity={Machine = "10.0.0.1"}} ]
```

## Inverting Criteria

The `Not` condition is satisifed (evaluates as `true`) when the term within it
evaluates to `false`.  This is a logical `NOT` operation.

In the example below, the critiera will evaluate as true if the session is NOT
a member of a hypothetical `admins` group.

```toml
criteria.Not = {Identity = { Group = 'admins' }}
```

## The Default ACL map

The default ACL map from the `main` branch at the time that this documentation
was built is included below, which may be different from the ACL in your
currently deployed version of KumoMTA.

```toml
--8<-- "acls/default.toml"
```
