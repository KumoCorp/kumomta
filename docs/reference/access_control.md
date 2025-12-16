# Access Control

{{since('dev')}}

This section describes how Access Control is implemented within KumoMTA.
There are three related parts to access control:

 * Authentication - deciding the identity of a session, which may be
   base on factors such as the peer address or an explicit credential
   exchanged as part of SMTP or HTTP authentication.
 * Authorization - deciding whether the session is allowed to perform some
   action on a resource based on their identity and the configured Access
   Control List for that resource.
 * Accounting - tracking the disposition of each authorization check for
   later review

These three areas are known collectively as **Authentication, Authorization and
Accounting**, or more succinctly as **AAA**.

## Authentication

Each session into the server has an associated
[AuthInfo](kumo.aaa/auth_info.md) object that records the authentication state.
It holds at least the following information:

 * The peer address - the IP address that is connecting to the server
 * A list of identities - for instance, the SMTP or HTTP auth username if auth
   was successfully processed.  If the list is empty we consider the session
   to be *Unauthenticated*.  If the list has at least one identity we consider
   the session to be *Authenticated*.
 * A list of group names - These may be established as part of authenticating
   the session, and in some cases, may be special pre-set groups assigned
   based on the peer address being present in the set of *trusted hosts*
   for the associated listener.

### HTTP Authentication

HTTP sessions are authenticated via the
[http_server_validate_auth_basic](events/http_server_validate_auth_basic.md)
event.

If that event returns `true` then the `username` parameter is added to the list
of identities in the `AuthInfo`.  Alternatively, the event can directly return
an `AuthInfo` to provide additional authentication information.

### SMTP Authentication

SMTP sessions are authenticated via the [smtp_server_auth_plain](events/smtp_server_auth_plain.md) event.

If that event returns `true` then the `authz` AND `authc` identities are both
added to the list of identities in the `AuthInfo`.  Alternatively, the event
can directly return an `AuthInfo` to provide additional authentication
information.

## Authorization

Authorization is the process of deciding whether a particular type of access is
permitted to happen to a particular resource.  In KumoMTA we encode
authorization rules into an *Access Control List* (ACL) for the resource.  To
aid in the definition of ACLs, a resource has an associated hierarchy and can
inherit ACLs from its parent/lineage in that hierarchy.

The following terms apply to authorization:

 * `resource` - a string that identifies the resource.  For example, it may be
   a string that corresponds to an HTTP API endpoint.
 * `privilege` - a string describing the nature of access that is desired.  For
   example, it may be `POST` to indicate an HTTP post request is being
   attempted, but it could be a more abstract privilege defined by the logic
   that gates access to the resource.
 * `criteria` - an expression term that is used to define when an ACL rule
   applies. For example, it might be "members of a specific named group".
 * `rule` - A combination of `privilege`, `criteria` and an access
   level that defines the access permitted for that combination of
   privilege and criteria.
 * `ACL` - An Access Control List, which is an ordered sequence of rules that
   apply to a specific resource.

When deciding whether access is permitted, the system will determine the
resource that is to be accessed and then attempt to resolve the ACL for that
resource.  This is carried out via the
[get_acl_definition](events/get_acl_definition.md) event, with a fall back to a
built in default ACL (which you can disable if you wish via
[kumo.aaa.set_fall_back_to_acl_map](kumo.aaa/set_fall_back_to_acl_map.md)).

Each rule in the returned ACL is considered (in the order in which they were
defined in the ACL) and compared against the `AuthInfo` for the session.  If
the `criteria` and `privilege` in the rule match then the access level defined
by the rule is taken as the definitive outcome and authorization checking stops
there at the first matching rule.

If no rules from the ACL match, then the parent resource of the current
resource is determined and its ACL is resolved, again evaluating its rules one
by one until a match is found.

If no matching rules were found, the parent resource of this resource is
resolved and the process is repeated until there are no more parent resources.
At that point the disposition is that access is **denied by default**.

### HTTP Resources

Access to HTTP API endpoints is decided by an initial authorization check in
the HTTP request routing layer based on the endpoint URL, HTTP method (which is
mapped to the `privilege`) and auth info in the current HTTP session.

When the AuthInfo for the HTTP session is instantiated and if the peer IP
address matches the
[start_http_listener.trusted_hosts](kumo/start_http_listener/trusted_hosts.md)
option then the built-in group `kumomta:http-listener-trusted-ip` is added to
the list of groups.

Rather than directly taking the HTTP URI from the request as the resource name,
the resource name is produced by re-composing the request elements.

For a URL like `http://localhost:8080/foo/bar/baz`:

 * The host portion of the request (`localhost:8080`) is ignored
 * The *listen* address of the listener (as per
   [start_http_listener.listen](kumo/start_http_listener/listen.md)) on which
   the request was received is used to decide which http listener is being
   accessed. For the sake of this example, let's assume that it is
   `127.0.0.1:8080`.
 * The HTTP request path is combined with the listen address to make a resource
   path of the form `http_listener/127.0.0.1:8080/foo/bar/baz`

The inheritance hierarchy for such a resource is shown below, with the first of
these being checked first and so on:

 * `http_listener/127.0.0.1:8080/foo/bar/baz`
 * `http_listener/127.0.0.1:8080/foo/bar`
 * `http_listener/127.0.0.1:8080/foo`
 * `http_listener/127.0.0.1:8080`
 * `http_listener/*/foo/bar/baz`
 * `http_listener/*/foo/bar`
 * `http_listener/*/foo`
 * `http_listener`

Notice that the path is traversed similarly to a filesystem path, but when we
reach the listener address (`http_listener/127.0.0.1:8080`) the next step is
the full resource path underneath a special `*` resource location
(`http_listener/*/foo/bar/baz`) which is consulted regardless of the listener
address.

Ultimately an HTTP access is checked against the logical `http_listener`
resource which can be used to encode a general access rule if required.

To illustrate how this works, the product default ACLs allow `GET`, `DELETE`
and `POST` access to members of the group `kumomta:http-listener-trusted-ip`
for the resource `http_listener/*/api/admin`.  Since all admin API endpoints
fall under this particular resource path, that one rule definition applies to
all admin API endpoints on all http listeners that may have been defined in the
product, with the effect being that any IP in the list of trusted hosts for the
listener is permitted to make admin API requests.

### Defining ACLs

If you wish to define your own ACLs then the recommended way is to deploy an
ACL map file to the system and configure it as shown in [Augmenting the default
ACL](events/get_acl_definition.md#augmenting-the-default-acl) or [Replacing the
default ACL](events/get_acl_definition.md#replacing-the-default-acl).

The documentation for [kumo.aaa.load_acl_map](kumo.aaa/load_acl_map.md)
describes the syntax of an ACL map file and includes a copy of the default ACL
map file.

It is also possible to dynamically load ACLs from other data sources, or
compute them dynamically, and that is demonstrated in [Advanced ACL
Building](events/get_acl_definition.md#advanced-acl-building).

## Accounting

You may enable and configure logging to an accounting log using
[kumo.aaa.configure_acct_log](kumo.aaa/configure_acct_log.md).

The accounting log configuration is similar to the delivery log configuration,
except that authentication and authorization events are logged rather than
delivery information.  You can choose whether to include or exclude successful
or failing authentication or authorization events.

