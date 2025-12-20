# add

```lua
kumo.api.admin.suppression.add { PARAMS }
```

Add or update a suppression entry. If the recipient already exists with the
same type and subaccount, the entry will be updated.

`PARAMS` is a lua table that can have the following keys:

## recipient

Required string. The email address to suppress.

## type

Required string. The type of suppression:

* `"transactional"` - Suppress transactional emails
* `"non_transactional"` - Suppress non-transactional/marketing emails

## source

Optional string. How the entry was added. Defaults to `"manual"`. Possible values:

* `"manual"` - Manually added
* `"bounce"` - Added due to a hard bounce
* `"complaint"` - Added due to a spam complaint
* `"list_unsubscribe"` - Added due to a list-unsubscribe request
* `"link_unsubscribe"` - Added due to a link unsubscribe action

## description

Optional string. Description or reason for the suppression.

## subaccount_id

Optional string. Tenant/subaccount identifier for multi-tenant setups.

## Example

```lua
-- Add a suppression due to spam complaint
kumo.api.admin.suppression.add {
    recipient = "user@example.com",
    type = "non_transactional",
    source = "complaint",
    description = "User clicked spam button"
}
```

## See Also

* [remove](remove.md) - Remove a suppression entry
* [POST /api/admin/suppression/v1](../http/api_admin_suppression_v1.md) - HTTP API equivalent
