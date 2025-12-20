# get

```lua
kumo.api.admin.suppression.get(recipient)
```

Retrieve all suppression entries for a specific recipient.

## recipient

Required string. The email address to look up.

## Returns

An array of suppression entry tables, each containing:

* `recipient` - The email address
* `suppression_type` - Either `"transactional"` or `"non_transactional"`
* `source` - How the entry was added
* `description` - Optional description
* `subaccount_id` - Optional subaccount identifier
* `created` - Timestamp when the entry was created
* `updated` - Timestamp when the entry was last updated

Returns an empty array if no entries are found.

## Example

```lua
local entries = kumo.api.admin.suppression.get("user@example.com")

for _, entry in ipairs(entries) do
    print(string.format("Suppressed for %s since %s",
        entry.suppression_type,
        entry.created))
end
```

## See Also

* [GET /api/admin/suppression/v1/{recipient}](../http/api_admin_suppression_get_v1.md) - HTTP API equivalent
