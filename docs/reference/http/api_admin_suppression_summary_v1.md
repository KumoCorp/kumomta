# `GET /api/admin/suppression/v1/summary`

Making a GET request to this endpoint returns summary statistics
about the suppression list.

The response is a json structure with the following format:

```json
{
    "total": 1500,
    "by_type": {
        "transactional": 300,
        "non_transactional": 1200
    },
    "by_source": {
        "manual": 500,
        "bounce": 400,
        "complaint": 600
    }
}
```

## Response Fields

### total

Integer. Total number of suppression entries.

### by_type

Object mapping each suppression type to its count.

### by_source

Object mapping each suppression source to its count.

## Example Usage

```console
$ curl http://127.0.0.1:8000/api/admin/suppression/v1/summary
{
    "total": 1500,
    "by_type": {
        "transactional": 300,
        "non_transactional": 1200
    },
    "by_source": {
        "manual": 500,
        "bounce": 400,
        "complaint": 600
    }
}
```

## Lua API

You can also get suppression summary in Lua policy scripts:

```lua
local stats = kumo.api.admin.suppression.summary()
print("Total suppressions:", stats.total)
print("Complaints:", stats.by_source.complaint or 0)
```

## See Also

* [GET /api/admin/suppression/v1](api_admin_suppression_list_v1.md) - List all suppression entries
* [POST /api/admin/suppression/v1/check](api_admin_suppression_check_v1.md) - Check suppression status
