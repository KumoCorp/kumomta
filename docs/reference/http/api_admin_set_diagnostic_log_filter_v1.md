# `POST /api/admin/set_diagnostic_log_filter/v1`

Changes the diagnostic log filter dynamically.

```console
$ curl -i 'http://localhost:8000/api/admin/set_diagnostic_log_filter/v1' \
    -H 'Content-Type: application/json' \
    -d '{"filter":"kumod=trace"}'
```

The above is equivalent to:

```lua
kumo.set_diagnostic_log_filter 'kumod=trace'
```

except that an administrator can execute this ad-hoc to dynamically
adjust the log filtering.

See [kumo.set_diagnostic_log_filter](../kumo/set_diagnostic_log_filter.md)
for more information about diagnostic log filters.

The body of the post request must be a JSON object with a `filter` field:

```json
{
    "filter": "kumod=trace"
}
```

## Kumo CLI

In addition to making raw API requests, you may use the kumo CLI:

```console
$ kcli --endpoint http://127.0.0.1:8000 set-log-filter 'kumod=trace'
OK
```

Run `kcli set-log-filter --help` for more informtion.
