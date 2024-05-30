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

and the response will look something like this, with an entry for
each matching queue name and the count of matching messages that
were bounced *so far*:

```json
{"bounced":{"gmail.com":42}, "total_bounced":42}
```

!!! note
    More recent builds of KumoMTA perform most of the bouncing
    asynchronously with respect to this bounce request being
    made, in order to make the overall system more responsive
    and performant. As a result, the numbers reported in the
    response to this command will often show as either zero
    or some smaller number than the total that will be affected.
    Use the [GET /api/admin/bounce/v1](api_admin_bounce_v1.md)
    API or the `kcli bounce-list` command to review the current
    totals.

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
$ kcli --endpoint http://127.0.0.1:8000 bounce --everything --reason purge
{
  "id": "0234c7c9-afd3-49f9-9a4c-a1cc37fcc53b",
  "bounced": {},
  "total_bounced": 0
}
```

Run `kcli bounce --help` for more informtion.
