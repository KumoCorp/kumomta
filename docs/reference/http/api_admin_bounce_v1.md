# `POST /api/admin/bounce/v1`

Making a POST request to this endpoint allows the system operator
to administratively bounce messages that match certain criteria,
or if no criteria are provided, ALL messages.

!!! danger
    There is no way to undo the actions carried out by this request!

The body of the post request must be a JSON object; here's an example:

```json
{
    "domain": "gmail.com",
    "reason": "no time to explain!11!"
}
```

and the response will look something like this, returning a unique
identifier for the bounce entry:

```json
{"id": "eab8cf70-4f64-4e02-9493-0b2f190a9a73"}
```

Use the [GET /api/admin/bounce/v1](api_admin_bounce_v1.md)
API or the `kcli bounce-list` command to review the current
totals for the bounces that have been registered in the system.

The following fields are possible in the request:

### domain

Optional string. The domain name to match.
If omitted, any domain will match.

### routing_domain

{{since('2023.08.22-4d895015', indent=True)}}
    Optional string. The routing_domain name to match.
    If omitted, any routing_domain will match.

### campaign

Optional string. The campaign name to match.
If omitted, any campaign will match.

### tenant

Optional string. The tenant to match.
If omitted, any tenant will match.

!!! danger
    If you specify none of `domain`, `campaign` or `tenant`, then
    *ALL* queues will be bounced.

    With great power, comes great responsibility!

### reason

Required. Reason to log in the delivery log.

### duration

Optional duration string. Defaults to `"5m"`.
Specifies how long this bounce directive remains active.

While active, newly injected messages that match the
bounce criteria will also be bounced.

## Kumo CLI

In addition to making raw API requests, you may use the kumo CLI:

```console
$ kcli bounce --everything --reason purge
NOTE: the bounce is running async. Use the bounce-list command to review ongoing status!
eab8cf70-4f64-4e02-9493-0b2f190a9a73
```

Run `kcli bounce --help` for more informtion.
