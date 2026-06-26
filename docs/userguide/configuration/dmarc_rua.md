# DMARC RUA Reports (Customer Guidance)

Receivers such as **Gmail** and **Microsoft Outlook/Hotmail** periodically send
**DMARC aggregate reports** (RUA) to the address in your domain’s DMARC DNS record
(`rua=mailto:...`). Those reports summarize how mail claiming your domain
authenticated (SPF/DKIM/alignment) and what disposition the receiver applied.

KumoMTA includes a **`policy-extras.dmarc_rua`** helper that:

1. Detects DMARC `<feedback>` XML in inbound report mail
2. Parses records (source IP, counts, policy evaluation, auth results)
3. Labels common reporters (Gmail, Microsoft, Yahoo, …)
4. Builds **customer-facing guidance**: what is wrong, severity, and recommended fixes
5. Optionally stores structured analysis + markdown briefing on message meta for webhooks/logs

This complements inbound [mail auth checks](../policy/mail_auth.md) (`policy-extras.mail_auth`),
which evaluate **your** authentication on received mail. RUA reports describe how
**remote ISPs** saw mail **sent as your customers’ domains**.

!!! note "Scope"
    This helper is an **operator/customer guidance** tool at report receipt time.
    It is not a full multi-month DMARC analytics platform (trending, inventory
    across all tenants). For enterprise-wide RUA analytics, many organizations
    also use specialized DMARC services; you can still land reports here and
    forward summaries via log hooks.

## DNS prerequisites

For each sending domain (customer or your brand):

```text
_dmarc.customer.example. IN TXT "v=DMARC1; p=none; rua=mailto:dmarc-rua@reports.example.com; fo=1"
```

Start with **`p=none`** while you fix sources. Tighten to `quarantine` / `reject`
only when legitimate pass rates are consistently high.

Point `rua=` at a mailbox/domain you accept on KumoMTA (or forward reports into it).

## Accept RUA mail

Use [listener domains](domains.md) so external reporters can deliver to your RUA address:

{% call toml_data() %}
["reports.example.com"]
# Accept mail addressed to this domain (reporters are not your relay_hosts)
relay_to = true
{% endcall %}

Some operators use a dedicated subdomain solely for `dmarc@` / `rua@` addresses.

## Configure the helper

Shipped defaults: `assets/policy-extras/dmarc_rua.toml` (reporter labels + options).

Operator file example `/opt/kumomta/etc/dmarc_rua.toml`:

{% call toml_data() %}
set_meta = true
set_guidance_meta = true
min_count_for_issue = 1
top_sources_limit = 20

# Optional: only analyze when recipient domain is one of these
# rua_domains = ["reports.example.com"]

# Label your known infrastructure so guidance distinguishes expected vs surprise sources
[known_sources."203.0.113.10"]
label = "KumoMTA customer-A pool-1"
expected = true

[known_sources."198.51.100.0/24"]
label = "ESP / partner range (document exact includes in SPF)"
expected = true
{% endcall %}

## Wire into policy

```lua
local dmarc_rua = require 'policy-extras.dmarc_rua'

local rua = dmarc_rua:setup {
  '/opt/kumomta/share/policy-extras/dmarc_rua.toml',
  '/opt/kumomta/etc/dmarc_rua.toml',
}

kumo.on('smtp_server_message_received', function(msg)
  local analysis = rua.process_message(msg)
  if not analysis then
    return -- not a DMARC report (or not for configured rua_domains)
  end

  if analysis.error then
    kumo.log_warn('dmarc_rua: ' .. tostring(analysis.error))
    return
  end

  -- Structured fields for automation / multi-tenant routing
  -- msg:get_meta('dmarc_rua_analysis')
  -- Human briefing for support / customer email templates:
  -- msg:get_meta('dmarc_rua_guidance')

  kumo.log_info(analysis.executive_summary)
end)
```

### Optional: webhook / log hook

Use [log hooks](../operation/webhooks.md) or your existing queue pipeline to POST
`dmarc_rua_analysis` JSON (or `dmarc_rua_guidance` markdown) to a customer portal,
ticketing system, or internal deliverability bot.

## What customers receive (conceptually)

Each processed report produces:

| Field | Purpose |
|-------|---------|
| `executive_summary` | One-line: reporter, domain, pass/fail volume, policy |
| `reporter` | Friendly ISP label (e.g. `Gmail / Google`) |
| `pass_rate_percent` | Share of messages with DMARC pass evaluation |
| `top_sources` | Highest-volume source IPs with SPF/DKIM/disposition |
| `guidance[]` | Prioritized issues with severity + recommended actions |
| `customer_briefing` | Markdown suitable for support copy/paste |

### Issue classes (examples)

| Issue key | Typical meaning | Customer direction |
|-----------|-----------------|-------------------|
| `dmarc_pass` | Aligned SPF and/or DKIM OK | Maintain; monitor trends |
| `spf_fail_dkim_pass` | SPF fail, DKIM pass | Often forwarding; prioritize DKIM everywhere |
| `spf_pass_dkim_fail` | SPF pass, DKIM fail/missing | Enable/fix DKIM on all senders |
| `both_fail` | Neither aligned pass | Unauthorized or misconfigured sender—fix or investigate spoofing |
| `disposition_quarantine` / `disposition_reject` | Receiver enforced policy | Urgent fix before more enforcement |
| `policy_none_monitoring` | `p=none` in published policy | Good for ramp-up; fix sources before tightening |
| `unknown_source` | IP not in your `known_sources` | Map to ESP/MTA or treat as abuse |

Guidance text is customizable under `[guidance.<key>]` in TOML if you want branded wording.

## Using reports to guide customers (playbook)

1. **Confirm the reporter** — Gmail vs Microsoft vs others may show different paths (forwarders, filters).
2. **Sort by count** — Fix the largest failing `source_ip` groups first (helper `top_sources`).
3. **Classify legitimacy** — Compare to `known_sources` and customer’s declared ESPs/KumoMTA pools.
4. **Authorize or stop** — Legitimate → SPF include / DKIM sign + align. Illegitimate → security/abuse path; don’t “fix” by allowing spoofers.
5. **Re-check alignment** — DMARC needs **aligned** pass; DKIM `d=` / SPF domain must align with `From` domain (relaxed vs strict per `adkim`/`aspf`).
6. **Only then tighten policy** — Move `p=none` → `quarantine` → `reject` after pass rates are healthy.
7. **Cross-check** — Google Postmaster Tools, Microsoft SNDS, complaint rates; RUA is authentication visibility, not full inbox placement.

## API (Lua)

```lua
local dmarc_rua = require 'policy-extras.dmarc_rua'

-- Without setup (defaults only):
local parsed, err = dmarc_rua.parse_aggregate_report_xml(xml_string)
local analysis = dmarc_rua.analyze_report(parsed, config_table)
local analysis, err = dmarc_rua.analyze_xml(xml_string, config_table)
local analysis = dmarc_rua.process_message(msg, config_table)

-- With setup (memoized config files):
local rua = dmarc_rua:setup { '.../dmarc_rua.toml', '/opt/kumomta/etc/dmarc_rua.toml' }
rua.process_message(msg)
rua.analyze_xml(xml_string)
rua.parse_xml(xml_string)
```

## Limitations (current)

* Parses standard aggregate `<feedback>` XML from message bodies/parts; exotic compression/encoding may need preprocessing.
* CIDR matching for `known_sources` is best-effort (exact IP or simple prefix); prefer listing concrete IPs for precision.
* Does not ingest RUF (forensic) samples end-to-end in this helper.
* No built-in multi-week rollup store—pair with your DB/webhook for history.

## See also

* [Listener domains](domains.md) — accept RUA mail
* [Mail authentication helper](../reference/policy-extras.mail_auth/check.md) — inbound SPF/DKIM/DMARC checks
* [Log hooks / webhooks](../operation/webhooks.md) — deliver briefings to external systems
* [IP warmup](ip_warmup.md) — volume ramp for new dedicated IPs (orthogonal to DMARC DNS policy)
* RFC 7489 / DMARC aggregate reports (RUA)
