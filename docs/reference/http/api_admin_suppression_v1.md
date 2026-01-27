# `POST /api/admin/suppression/v1`

Making a POST request to this endpoint allows the system operator
to create or update a suppression list entry. If the recipient
already exists with the same type and subaccount, the entry will
be updated; otherwise, a new entry is created.

The suppression list prevents emails from being sent to specific recipients.

!!! note
     By default, the suppression list uses in-memory storage and data is lost
     on restart. For persistent storage, configure RocksDB using
     [kumo.configure_suppression_store](../kumo/configure_suppression_store.md).

The body of the post request must be a JSON object; here's an example:

```json
{
    "recipient": "user@example.com",
    "type": "non_transactional",
    "source": "complaint",
    "description": "User marked email as spam"
}
```

and the response will look something like this:

```json
{
    "created": 1,
    "updated": 0,
    "errors": []
}
```

The following fields are possible in the request:

### recipient

Required string. The email address to suppress.

### type

Required string. The type of suppression. Must be one of:

* `transactional` - Suppress transactional emails (order confirmations, password resets, etc.)
* `non_transactional` - Suppress non-transactional/marketing emails

### source

Optional string. How the entry was added to the suppression list.
Defaults to `manual` for API calls. Possible values:

* `manual` - Manually added by an administrator or API call
* `bounce` - Added due to a hard bounce
* `complaint` - Added due to a spam complaint (FBL)
* `list_unsubscribe` - Added due to a list-unsubscribe request
* `link_unsubscribe` - Added due to a link unsubscribe action

### description

Optional string. A description or reason for the suppression.

### subaccount_id

Optional string. Tenant/subaccount identifier for multi-tenant setups.

## Example Usage

```console
$ curl -X POST http://127.0.0.1:8000/api/admin/suppression/v1 \
-H "Content-Type: application/json" \
-d '{
    "recipient": "user@example.com",
    "type": "non_transactional",
    "description": "User opted out of marketing"
}'
{"created":1,"updated":0,"errors":[]}
```

## See Also

* [GET /api/admin/suppression/v1](api_admin_suppression_list_v1.md) - List suppression entries
* [DELETE /api/admin/suppression/v1](api_admin_suppression_delete_v1.md) - Delete suppression entries
* [POST /api/admin/suppression/v1/check](api_admin_suppression_check_v1.md) - Check suppression status
* [POST /api/admin/suppression/v1/bulk](api_admin_suppression_bulk_v1.md) - Bulk create entries
