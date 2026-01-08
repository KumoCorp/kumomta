# `GET /api/admin/suppression/v1`

Making a GET request to this endpoint allows the system operator
to list all suppression entries with optional filtering.

The response is a json structure with the following format:

```json
{
    "results": [
        {
            "recipient": "user@example.com",
            "type": "non_transactional",
            "source": "complaint",
            "description": "User marked email as spam",
            "created": "2024-01-15T10:30:00Z",
            "updated": "2024-01-15T10:30:00Z",
            "subaccount_id": null
        }
    ],
    "total_count": 1,
    "next_cursor": null
}
```

## Query Parameters

### recipient

Optional string. Filter by email address (partial match supported).

### type

Optional string. Filter by suppression type (`transactional` or `non_transactional`).

### source

Optional string. Filter by source (`manual`, `bounce`, `complaint`, `list_unsubscribe`, `link_unsubscribe`).

### subaccount_id

Optional string. Filter by subaccount identifier.

### from

Optional ISO 8601 timestamp. Filter by entries created after this time.

### to

Optional ISO 8601 timestamp. Filter by entries created before this time.

### limit

Optional integer. Maximum number of results to return.
Default: 1000, Maximum: 10000.

### cursor

Optional string. Pagination cursor returned from a previous request.

## Example Usage

```console
$ curl "http://127.0.0.1:8000/api/admin/suppression/v1?type=non_transactional&limit=100"
{
    "results": [
        {
            "recipient": "user@example.com",
            "type": "non_transactional",
            "source": "complaint",
            "description": "User marked email as spam",
            "created": "2024-01-15T10:30:00Z",
            "updated": "2024-01-15T10:30:00Z",
            "subaccount_id": null
        }
    ],
    "total_count": 1,
    "next_cursor": null
}
```

## See Also

* [POST /api/admin/suppression/v1](api_admin_suppression_v1.md) - Create/update suppression entries
* [GET /api/admin/suppression/v1/{recipient}](api_admin_suppression_get_v1.md) - Get entries for a specific recipient
