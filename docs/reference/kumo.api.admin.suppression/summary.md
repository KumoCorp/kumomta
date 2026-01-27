# summary

```lua
kumo.api.admin.suppression.summary()
```

Get summary statistics for the suppression list.

## Returns

A table with:

* `total` - Total number of suppression entries
* `by_type` - Table mapping suppression type to count
* `by_source` - Table mapping source to count

## Example

```lua
local stats = kumo.api.admin.suppression.summary()

print(string.format("Total suppressions: %d", stats.total))

for type_name, count in pairs(stats.by_type) do
    print(string.format("  %s: %d", type_name, count))
end

for source, count in pairs(stats.by_source) do
    print(string.format("  %s: %d", source, count))
end
```

## See Also

* [GET /api/admin/suppression/v1/summary](../http/api_admin_suppression_summary_v1.md) - HTTP API equivalent
