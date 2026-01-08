# list

```lua
kumo.api.admin.suppression.list { PARAMS }
```

List suppression entries with optional filtering.

`PARAMS` is a lua table that can have the following keys:

## recipient

Optional string. Filter by recipient email (partial match supported).

## type

Optional string. Filter by suppression type:

* `"transactional"` - Show only transactional suppressions
* `"non_transactional"` - Show only non-transactional suppressions

## source

Optional string. Filter by source.

## limit

Optional number. Maximum number of entries to return. Default: 1000, Max: 10000.

## Returns

A table with:

* `results` - Array of suppression entries
* `total_count` - Total number of matching entries

## Example

```lua
local result = kumo.api.admin.suppression.list {
    type = "non_transactional",
    limit = 100
}

print(string.format("Found %d entries", result.total_count))

for _, entry in ipairs(result.results) do
    print(entry.recipient)
end
```

## See Also

* [GET /api/admin/suppression/v1](../http/api_admin_suppression_list_v1.md) - HTTP API equivalent
