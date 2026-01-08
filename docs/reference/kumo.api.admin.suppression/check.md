# check

```lua
kumo.api.admin.suppression.check { PARAMS }
```

Check if one or more recipients are suppressed.

`PARAMS` is a lua table that can have the following keys:

## recipients

Required array of strings. The email addresses to check.

## type

Optional string. If specified, only check for this suppression type:

* `"transactional"` - Check for transactional suppression
* `"non_transactional"` - Check for non-transactional suppression

If not specified, checks for any suppression type.

## subaccount_id

Optional string. Tenant/subaccount identifier for multi-tenant setups.

## Returns

A table with a `results` field containing a map of recipient email to boolean
indicating whether the recipient is suppressed.

## Example

```lua
local result = kumo.api.admin.suppression.check {
recipients = {"user1@example.com", "user2@example.com"},
type = "non_transactional"
}

-- result.results["user1@example.com"] will be true or false
if result.results["user1@example.com"] then
print("user1@example.com is suppressed")
end
```

## See Also

* [POST /api/admin/suppression/v1/check](../http/api_admin_suppression_check_v1.md) - HTTP API equivalent
