# remove

```lua
kumo.api.admin.suppression.remove { PARAMS }
```

Remove a suppression entry.

`PARAMS` is a lua table that can have the following keys:

## recipient

Required string. The email address to remove from suppression.

## type

Optional string. The type of suppression to remove:

* `"transactional"` - Remove transactional suppression
* `"non_transactional"` - Remove non-transactional suppression

If not specified, removes all suppression types for the recipient.

## subaccount_id

Optional string. Tenant/subaccount identifier for multi-tenant setups.

## Returns

A table with:

* `deleted` - Number of entries deleted

## Example

```lua
-- Remove non-transactional suppression for a recipient
local result = kumo.api.admin.suppression.remove {
    recipient = "user@example.com",
    type = "non_transactional"
}

print(string.format("Deleted %d entries", result.deleted))
```

## See Also

* [add](add.md) - Add a suppression entry
* [DELETE /api/admin/suppression/v1](../http/api_admin_suppression_delete_v1.md) - HTTP API equivalent
