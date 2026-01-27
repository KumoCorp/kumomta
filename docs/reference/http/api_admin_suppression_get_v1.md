# `GET /api/admin/suppression/v1/{recipient}`

Making a GET request to this endpoint allows the system operator
to retrieve all suppression entries for a specific email address.

## Path Parameters

### recipient

Required string. The email address to look up.

## Response

The response is an array of suppression entries for the given recipient:

```json
[
    {
        "recipient": "user@example.com",
        "type": "non_transactional",
        "source": "complaint",
        "description": "User marked email as spam",
        "created": "2024-01-15T10:30:00Z",
        "updated": "2024-01-15T10:30:00Z",
        "subaccount_id": null
    },
    {
        "recipient": "user@example.com",
        "type": "transactional",
        "source": "bounce",
        "description": "Hard bounce - mailbox does not exist",
        "created": "2024-01-14T08:00:00Z",
        "updated": "2024-01-14T08:00:00Z",
        "subaccount_id": null
    }
]
```

If no entries are found, a 404 response is returned.

## Example Usage

```console
$ curl "http://127.0.0.1:8000/api/admin/suppression/v1/user@example.com"
[
    {
        "recipient": "user@example.com",
        "type": "non_transactional",
        "source": "complaint",
        "description": "User marked email as spam",
        "created": "2024-01-15T10:30:00Z",
        "updated": "2024-01-15T10:30:00Z",
        "subaccount_id": null
    }
]
```

## See Also

* [GET /api/admin/suppression/v1](api_admin_suppression_list_v1.md) - List all suppression entries
* [POST /api/admin/suppression/v1](api_admin_suppression_v1.md) - Create/update suppression entries
