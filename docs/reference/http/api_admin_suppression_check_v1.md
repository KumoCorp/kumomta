# `POST /api/admin/suppression/v1/check`

Making a POST request to this endpoint allows the system operator
to check if specific recipients are suppressed.

This is useful for validating a list of recipients before sending,
or for checking suppression status during message processing.

The body of the request must be a JSON object; here's an example:

```json
{
    "recipients": ["user1@example.com", "user2@example.com", "user3@example.com"],
    "type": "non_transactional"
}
```

and the response will look something like this:

```json
{
    "results": {
        "user1@example.com": true,
        "user2@example.com": false,
        "user3@example.com": false
    }
}
```

## Request Fields

### recipients

Required array of strings. The email addresses to check.

### type

Optional string. The type of suppression to check for (`transactional` or `non_transactional`).
If omitted, checks for any suppression type.

### subaccount_id

Optional string. Tenant/subaccount identifier for multi-tenant setups.

## Response Fields

### results

Object mapping each recipient to their suppression status:
* `true` - The recipient is suppressed
* `false` - The recipient is not suppressed

## Example Usage

```console
$ curl -X POST http://127.0.0.1:8000/api/admin/suppression/v1/check \
-H "Content-Type: application/json" \
-d '{
    "recipients": ["user1@example.com", "user2@example.com"],
    "type": "non_transactional"
}'
{"results":{"user1@example.com":true,"user2@example.com":false}}
```

## Lua API

You can also check suppression status in Lua policy scripts:

```lua
local is_suppressed = kumo.api.admin.suppression.check(
    "user@example.com",
    "transactional",  -- optional: suppression type
    nil               -- optional: subaccount_id
)

if is_suppressed then
    -- Handle suppressed recipient
end
```

## See Also

* [GET /api/admin/suppression/v1](api_admin_suppression_list_v1.md) - List all suppression entries
* [GET /api/admin/suppression/v1/summary](api_admin_suppression_summary_v1.md) - Get statistics
