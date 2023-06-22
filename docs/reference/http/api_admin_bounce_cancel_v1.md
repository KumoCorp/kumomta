# `DELETE /api/admin/bounce/v1`

Making a DELETE request to this endpoint allows the system operator
to delete an administrative bounce entry by its id.

The body of the request must have the following form:

```json
{
    "id": "169c3dc0-6518-41ef-bfbb-1f0ae426cb32"
}
```

If the id is invalid (or not longer active), then a `404` status will be returned.

## Kumo CLI

In addition to making raw API requests, you may use the kumo CLI:

```console
$ kcli --endpoint http://127.0.0.1:8000 bounce-cancel --id 169c3dc0-6518-41ef-bfbb-1f0ae426cb32
removed 0234c7c9-afd3-49f9-9a4c-a1cc37fcc53b
```

Run `kcli bounce-cancel --help` for more informtion.
