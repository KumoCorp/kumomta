# `DELETE /api/admin/suppression/v1`

Making a DELETE request to this endpoint allows the system operator
to remove a suppression entry for a specific recipient.

The body of the request must be a JSON object; here's an example:

```json
{
    "recipient": "user@example.com",
    "type": "non_transactional"
}
```

and the response will look something like this:

```json
{
    "deleted": 1,
    "errors": []
}
```

The following fields are possible in the request:

### recipient

Required string. The email address to remove from suppression.

### type

Optional string. The type of suppression to remove (`transactional` or `non_transactional`).
If omitted, all suppression types for this recipient are removed.

### subaccount_id

Optional string. Tenant/subaccount identifier for multi-tenant setups.

## Example Usage

```console
$ curl -X DELETE http://127.0.0.1:8000/api/admin/suppression/v1 \
-H "Content-Type: application/json" \
-d '{
    "recipient": "user@example.com",
    "type": "non_transactional"
}'
{"deleted":1,"errors":[]}
```

## See Also

* [POST /api/admin/suppression/v1/bulk/delete](api_admin_suppression_bulk_delete_v1.md) - Bulk delete entries
* [POST /api/admin/suppression/v1](api_admin_suppression_v1.md) - Create/update suppression entries
