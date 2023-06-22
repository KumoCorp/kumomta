# `GET /api/admin/bounce/v1`

Making a GET request to this endpoint allows the system operator
to list all currently active administrative bounces that have been
configured.

The response is a json structure with the following format:

```json
[
  {
    "id": "169c3dc0-6518-41ef-bfbb-1f0ae426cb32",
    "campaign": null,
    "tenant": null,
    "domain": null,
    "reason": "purge",
    "duration": "4m 50s 207ms 320us 231ns",
    "bounced": {
      "wezfurlong.org": 1
    },
    "total_bounced": 1
  }
]
```

Each entry of the array shows the bounce entry and its matching
criteria, along with an `id` that can be used to cancel the
entry, the map of queue name to the number of bounced messages
and the overall number of bounced messages.

The remaining duration of the entry is also included.

## Kumo CLI

In addition to making raw API requests, you may use the kumo CLI:

```console
$ kcli --endpoint http://127.0.0.1:8000 bounce-list
[
  {
    "id": "169c3dc0-6518-41ef-bfbb-1f0ae426cb32",
    "campaign": null,
    "tenant": null,
    "domain": null,
    "reason": "purge",
    "duration": "4m 50s 207ms 320us 231ns",
    "bounced": {
      "wezfurlong.org": 1
    },
    "total_bounced": 1
  }
]
```

Run `kcli bounce-list --help` for more informtion.
