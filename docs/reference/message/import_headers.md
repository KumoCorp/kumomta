---
tags:
 - meta
---

# import_headers

```
message:import_headers(SPECS)
```

{{since('dev')}}

Iterates the headers of the message, importing matching header values into the
message metadata. `SPECS` is an array of tables, one per header name or pattern,
each describing how that header should be matched, named in the metadata, and
optionally removed.

This is a more flexible alternative to
[message:import_x_headers](import_x_headers.md). The same call can be used to
import multiple distinct headers with different rules, and can also be used to
strip the matched headers from the message body in a single pass.

## Spec fields

Each entry in `SPECS` accepts the following fields:

* `name` *(required, string)* — the header name to match.

  Matching is always case-insensitive. The pattern can be either a precise
  header name (e.g. `"Subject"`) or a name with a single trailing `*`
  wildcard (e.g. `"X-*"`), which matches any header whose name begins with the
  literal portion of the pattern. Bare `*`, leading wildcards, and interior
  wildcards are not supported.

* `match` *(optional, string, default `"last"`)* — which matching headers to
  capture:

    * `"first"` — capture the first matching header value as a string.
    * `"last"` — capture the last matching header value as a string.
    * `"all"` — capture every matching header value as an array of strings.

* `transform` *(optional, string, default `"snake_case"`)* — how to derive the
  metadata key from the matched header name. All transforms produce a
  deterministic key. Examples below assume a matched header named
  `X-Campaign-Id`:

    * `"snake_case"` — `x_campaign_id` (matches the transform performed by
      [message:import_x_headers](import_x_headers.md))
    * `"kebab_case"` — `x-campaign-id`
    * `"camel_case"` — `xCampaignId`
    * `"pascal_case"` — `XCampaignId`

* `target` *(optional, string)* — explicitly set the metadata key to use,
  bypassing `transform`. Only valid when `name` is a precise header name; using
  `target` with a wildcard pattern produces an error.

* `remove` *(optional, bool, default `false`)* — when `true`, the matched
  header instances are removed from the message body after their values have
  been captured.

If a spec produces no matches, no metadata is written for it (including for
`match = "all"`, which writes nothing rather than an empty array).

When more than one spec could match a header, the *first* spec in `SPECS` that
matches wins. This lets you place a precise rule for a specific header in
front of a broader wildcard catch-all.

## Examples

Import all `X-` headers into metadata, and strip them from the message:

```lua
msg:import_headers {
  { name = 'X-*', remove = true },
}
```

Capture every `Received:` header as an array, while also lifting the subject:

```lua
msg:import_headers {
  { name = 'Received', match = 'all' },
  { name = 'Subject' },
}
```

Treat one specific header specially, while still importing the rest of the
`X-` headers with the default transform:

```lua
msg:import_headers {
  { name = 'X-Campaign-Id', target = 'campaign_id' },
  { name = 'X-*' },
}
```

Use a different naming style:

```lua
msg:import_headers {
  { name = 'X-*', transform = 'camel_case' },
}
-- X-Campaign-Id is captured as `xCampaignId`
```

## See Also

* [message:import_x_headers](import_x_headers.md)
* [message:remove_x_headers](remove_x_headers.md)
