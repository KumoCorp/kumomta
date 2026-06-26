# IP Warmup (Automatic Volume Ramp)

New or cold dedicated sending IPs should not send at full production volume on
day one. Mailbox providers evaluate reputation partly from IP history; a sudden
spike from an unknown IP commonly triggers throttling, deferrals, junk placement,
or blocks.

KumoMTA ships an **IP warmup policy helper** (`policy-extras/warmup.lua`) that
automatically applies a day-by-day **overall per-source volume ramp** using the
existing egress-path rate controls. You enroll each source once with a start
date; the helper selects the correct limit for “warmup day N” without you editing
rates every morning.

!!! important "What this feature is (and is not)"
    This helper implements **infrastructure volume ramp** only: it limits how
    often a given egress source can be selected for delivery across destinations.

    It does **not**:

    * Improve engagement, opens, or replies (no “inbox warmup” / engagement farming)
    * Fix weak authentication, bad lists, or high complaint rates
    * Replace traffic shaping for Gmail/Yahoo/Microsoft site limits ([Traffic Shaping](trafficshaping.md))
    * Auto-pause on reputation signals (planned as a future phase; use TSA/manual `status = "paused"` today)

    Warmup is necessary for dedicated IPs but **not sufficient** for inbox placement.
    Maintain SPF/DKIM/DMARC alignment, permission-based lists, low complaints, and
    monitor Google Postmaster Tools and Microsoft SNDS.

## How it works

1. You define named **schedules** (day number → daily limit) in TOML. Shipped
   presets cover common cases (`conservative`, `aggressive`, `transactional`).
2. You **enroll** each warming source with `warmup_start = "YYYY-MM-DD"` and an
   optional `schedule` name.
3. In `get_egress_path_config`, the helper injects throttles into
   [`additional_source_selection_rates`](../../reference/kumo/make_egress_path/additional_source_selection_rates.md):
   * `warmup-source-<name>` — overall daily selection limit for that IP/source
   * `warmup-source-<name>-hourly` — optional hourly spread (default on), derived
     from daily limit ÷ `active_sending_hours`
4. After the last schedule day, warmup stops applying limits (unless you set
   `hold_final_day` or `post_warmup_rate`). Normal shaping and pool weights still apply.

This complements (does not replace) pool [weights](sendingips.md) and per-site
[`source_selection_rate`](../../reference/kumo/make_egress_path/source_selection_rate.md)
rules in shaping config.

## Quick start

### 1. Create operator warmup config

Copy or layer the shipped presets. On a package install the community file is
typically at `/opt/kumomta/share/policy-extras/warmup.toml`. Create your local
file (example `/opt/kumomta/etc/warmup.toml`):

{% call toml_data() %}
# Use shipped presets by loading them first (see init.lua below).
# Only list sources you want to ramp.

[source."ip-3"]
warmup_start = "2026-06-01"
schedule = "conservative"

# Optional: mark done early, pause, or hold at final day rate
# status = "complete"
# status = "paused"
# hold_final_day = true
# post_warmup_rate = "500000/day,max_burst=100"
{% endcall %}

### 2. Wire the helper in `init.lua`

Compose with the shaping helper (recommended). Always pass `skip_make = true` into
shaping first, apply warmup, then call `kumo.make_egress_path`—or use `wrap()`:

```lua
local shaping = require 'policy-extras.shaping'
local warmup = require 'policy-extras.warmup'

local shaper = shaping:setup_with_automation {
  extra_files = { '/opt/kumomta/etc/policy/shaping.toml' },
  publish = { 'http://127.0.0.1:8008' },
  subscribe = { 'http://127.0.0.1:8008' },
}

local warmer = warmup:setup {
  '/opt/kumomta/share/policy-extras/warmup.toml',
  '/opt/kumomta/etc/warmup.toml',
}

kumo.on('get_egress_path_config', warmer.wrap(shaper.get_egress_path_config))
```

Manual compose (equivalent):

```lua
kumo.on('get_egress_path_config', function(domain, source, site)
  local params = shaper.get_egress_path_config(domain, source, site, true)
  warmer.apply_to_params(params, source)
  return kumo.make_egress_path(params)
end)
```

Sources **not** listed under `[source."..."]` in warmup config are unchanged.

### 3. Keep sources/pools defined separately

Define the egress source and pool with the [sources helper](sendingips.md) as usual.
Warmup config only controls **rates for enrolled source names**; it does not create IPs.

## Built-in schedule presets

| Preset | Length (approx.) | Intended use |
|--------|------------------|--------------|
| `conservative` (default) | ~30 days | New dedicated IPs, marketing/mixed permission lists |
| `aggressive` | ~14 days | Strong existing **domain** reputation; mainly transactional. Higher risk on cold lists |
| `transactional` | ~21 days | Opt-in transactional/lifecycle mail; higher early volume than `conservative` |

Exact day tables live in [`assets/policy-extras/warmup.toml`](https://github.com/KumoCorp/kumomta/blob/main/assets/policy-extras/warmup.toml).
Treat numbers as **starting guidance**, not a guarantee of inbox placement.

Choose preset per source:

{% call toml_data() %}
[source."tx-ip-1"]
warmup_start = "2026-06-01"
schedule = "transactional"
{% endcall %}

## Custom schedules and overrides

### Global options (any warmup file layer)

| Option | Default | Meaning |
|--------|---------|---------|
| `default_schedule` | `conservative` | Schedule name when source omits `schedule` |
| `active_sending_hours` | `12` | Used to derive hourly limit: `ceil(daily / hours)` |
| `max_burst` | `1` | Applied when limits are integers (spreads sends; recommended for warmup) |
| `apply_hourly_spread` | `true` | Add hourly selection throttle |
| `hold_final_day` | `false` | If true, stay on last schedule day forever |
| `before_start_behavior` | `day1` | If now &lt; `warmup_start`: `day1` or `block` (`0/day`) |
| `timezone` | `UTC` | Day boundaries use UTC midnight today; non-UTC warns at validate time |

### Per-source options (`[source."name"]`)

| Option | Meaning |
|--------|---------|
| `warmup_start` | Required when `status` is `active`. Calendar date `YYYY-MM-DD` (day 1) |
| `schedule` | Preset or custom schedule name |
| `status` | `active` (default), `complete` (no ramp; optional `post_warmup_rate`), `paused` (`paused_rate` or `0/day`) |
| `day_offset` | Add/subtract days (e.g. resume ramp mid-schedule) |
| `hold_final_day` | Override global |
| `apply_hourly_spread` | Override global |
| `active_sending_hours` | Override global |
| `max_burst` | Override global for integer limits |
| `post_warmup_rate` | Throttle applied after schedule ends (or with `status = "complete"`) |
| `paused_rate` | Used when `status = "paused"` |
| `extra_selection_rates` | Map of extra named throttles merged into path config |
| `before_start_behavior` | Override global |

### Custom schedule tables

{% call toml_data() %}
[schedule.my_brand]
1 = 20
2 = 40
3 = 80
4 = 150
5 = 300
# Or full throttle strings:
# 6 = "500/day,max_burst=1"

[source."ip-4"]
warmup_start = "2026-06-10"
schedule = "my_brand"
{% endcall %}

Layer files: pass shipped presets first, operator file second, so local
`[schedule.conservative]` entries override individual days without copying the
whole preset.

## Operational guidance

1. **Start with best recipients** — engaged subscribers first; ramp content/list segments as volume grows (policy/campaign design, not this helper).
2. **Separate streams** — transactional vs marketing on different sources/pools so one stream cannot burn the other.
3. **Monitor daily** — bounces, complaints, GPT/SNDS, blocks. If metrics worsen, set `status = "paused"` or reduce via a custom schedule / `day_offset` backward—not only “push through.”
4. **Consistency** — long gaps in sending can set reputation back; the helper does not detect gaps (config is date-based only). Adjust `warmup_start` or `day_offset` consciously if you restart.
5. **Validate config** — run `kumod --policy ... --validate` after edits; the helper checks dates, schedule names, and status values.
6. **Shaping still applies** — site/provider limits from `shaping.toml` and TSA remain; warmup adds overall IP caps on top.

## Manual alternatives (without the helper)

Before this helper, operators typically:

* Lower pool [weight](sendingips.md) for a warming IP relative to established IPs
* Hand-maintain `source_selection_rate` / `provider_source_selection_rate` in shaping by day

Those approaches still work and can be combined with automatic warmup (e.g. weight for preference, helper for hard daily caps). Prefer the helper for day-indexed ramps so limits advance without daily config commits.

## Roadmap (not in current release)

* Provider-specific stricter ramps (e.g. extra Microsoft/Gmail constraints)
* Reputation-driven auto pause/slowdown integrated with TSA / bounce classification
* kcli/metrics exposing effective warmup day and remaining quota

## See also

* [Sending IPs / sources helper](sendingips.md)
* [Traffic shaping](trafficshaping.md)
* [`additional_source_selection_rates`](../../reference/kumo/make_egress_path/additional_source_selection_rates.md)
* [`source_selection_rate`](../../reference/kumo/make_egress_path/source_selection_rate.md)
