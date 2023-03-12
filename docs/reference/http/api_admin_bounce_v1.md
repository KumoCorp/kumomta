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
were bounced:

```json
{"bounced":{"gmail.com":42}, "total_bounced":42}
```

The following fields are possible in the request:

## domain

Optional string. The domain name to match.
If omitted, any domain will match.

## campaign

Optional string. The campaign name to match.
If omitted, any campaign will match.

## tenant

Optional string. The tenant to match.
If omitted, any tenant will match.

!!! danger
    If you specify none of `domain`, `campaign` or `tenant`, then
    *ALL* queues will be bounced.

    With great power, comes great responsibility!

## reason

Required. Reason to log in the delivery log.

## duration

Optional duration string. Defaults to `"5m"`.
Specifies how long this bounce directive remains active.

While active, newly injected messages that match the
bounce criteria will also be bounced.

