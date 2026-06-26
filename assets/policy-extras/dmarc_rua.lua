local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

--[[
DMARC RUA (aggregate report) policy helper.

Receivers such as Gmail and Outlook/Microsoft send daily XML aggregate reports
to the address in your DMARC DNS `rua=` tag. This helper:

  1. Detects and extracts DMARC aggregate report XML from inbound mail
     (including common gzip/zip attachments when the body is XML).
  2. Parses report metadata, published policy, and per-source records.
  3. Classifies issues (SPF/DKIM/alignment/DMARC fail) with ISP-aware labels.
  4. Produces customer-facing guidance you can log, webhook, or surface in ops tools.

This does NOT replace commercial DMARC platforms for long-term trending; it gives
KumoMTA operators and their customers actionable summaries at the point of receipt.

Usage (typical):

1. DNS: `_dmarc.customer.example` includes
   `rua=mailto:dmarc-rua@reports.example.com`

2. listener_domains.toml:
   ["reports.example.com"]
   relay_to = true   # or accept for local processing only

3. init.lua:
```lua
local dmarc_rua = require 'policy-extras.dmarc_rua'
local rua = dmarc_rua:setup {
  '/opt/kumomta/share/policy-extras/dmarc_rua.toml',
  '/opt/kumomta/etc/dmarc_rua.toml',
}

kumo.on('smtp_server_message_received', function(msg)
  local analysis = rua.process_message(msg)
  if analysis and analysis.is_dmarc_report then
    -- optional: enqueue webhook, write to DB, etc.
    -- kumo.log_info(kumo.json_encode(analysis.guidance))
  end
end)
```

See docs/userguide/configuration/dmarc_rua.md
]]

local DEFAULT_SHIPPED = '/opt/kumomta/share/policy-extras/dmarc_rua.toml'

-- Built-in guidance templates (overridable via [guidance.*] in toml)
local DEFAULT_GUIDANCE = {
  dmarc_pass = {
    title = 'DMARC pass',
    customer_summary = 'This traffic passed DMARC (aligned SPF and/or DKIM succeeded). No authentication action required for these sends.',
    severity = 'info',
    actions = {
      'Continue monitoring volume trends from this source.',
    },
  },
  spf_fail_dkim_pass = {
    title = 'SPF failed but DKIM passed (often forwarding or multi-hop)',
    customer_summary = 'Receivers saw SPF fail while DKIM still aligned. Common with mailing lists and forwarders. DMARC can still pass on DKIM alone.',
    severity = 'low',
    actions = {
      'Confirm DKIM is signed on all outbound paths for this domain.',
      'If volume is from forwarders only, this may be acceptable; prioritize DKIM alignment.',
      'Review whether SPF includes all direct sending IPs/ESPs you control.',
    },
  },
  spf_pass_dkim_fail = {
    title = 'SPF passed but DKIM failed or missing',
    customer_summary = 'Mail authenticated via SPF but DKIM did not pass/align. Forwarding is more fragile without DKIM; tighten signing on all platforms that send as this domain.',
    severity = 'medium',
    actions = {
      'Enable DKIM signing for every ESP, CRM, and MTA that sends as this From domain.',
      'Publish the correct selector DNS TXT at selector._domainkey.domain.',
      'Ensure relaxed alignment (adkim=r) unless you intentionally use strict.',
    },
  },
  both_fail = {
    title = 'SPF and DKIM both failed DMARC evaluation',
    customer_summary = 'Receivers classified this mail as failing DMARC. If p=quarantine/reject, delivery impact is likely. This is either unauthorized mail or a legitimate source not yet authorized/signed.',
    severity = 'high',
    actions = {
      'Identify the source_ip (WHOIS / your ESP IP ranges / KumoMTA egress IPs).',
      'If legitimate: add SPF ip4/ip6/include and/or fix DKIM signing + alignment.',
      'If not legitimate: consider tighter DMARC policy once legitimate sources are clean; investigate spoofing.',
      'Cross-check Google Postmaster Tools and Microsoft SNDS for the same period.',
    },
  },
  spf_fail_only_eval = {
    title = 'DMARC evaluated SPF as fail',
    customer_summary = 'Aligned SPF did not pass for this traffic at the reporting ISP.',
    severity = 'medium',
    actions = {
      'Verify sending IP is covered by SPF for the domain in policy_published.',
      'Stay under 10 DNS lookups in SPF; fix permerror/syntax issues.',
      'Use DKIM as backup so DMARC can pass when SPF breaks on forward.',
    },
  },
  dkim_fail_only_eval = {
    title = 'DMARC evaluated DKIM as fail',
    customer_summary = 'Aligned DKIM did not pass for this traffic at the reporting ISP.',
    severity = 'medium',
    actions = {
      'Verify DKIM signatures are applied before send on this source.',
      'Confirm selector/public key DNS matches the private key in use.',
      'Check for body/header modifications after signing (some relays break DKIM).',
    },
  },
  disposition_reject = {
    title = 'Receiver applied reject disposition',
    customer_summary = 'At least some volume was rejected under the domain DMARC policy. Fix authentication for legitimate sources urgently.',
    severity = 'critical',
    actions = {
      'Prioritize sources with highest message counts in this report.',
      'Do not raise DMARC enforcement further until pass rates recover.',
    },
  },
  disposition_quarantine = {
    title = 'Receiver applied quarantine disposition',
    customer_summary = 'Some mail was quarantined (often junk) due to DMARC policy. Treat as delivery risk for affected streams.',
    severity = 'high',
    actions = {
      'Fix SPF/DKIM/alignment for top failing sources before increasing send volume.',
    },
  },
  unknown_source = {
    title = 'Traffic from source not in your known_sources list',
    customer_summary = 'Report includes sends from an IP/range you have not labeled as expected. Could be a new ESP, partner, or abuse.',
    severity = 'medium',
    actions = {
      'Map source_ip to owner (ESP docs, cloud provider, or your MTA pool).',
      'If expected: add to known_sources in dmarc_rua.toml and authorize via SPF/DKIM.',
      'If unexpected: investigate as potential spoofing or compromised account.',
    },
  },
  policy_none_monitoring = {
    title = 'Published policy is p=none (monitor mode)',
    customer_summary = 'Domain is collecting reports without enforcement. Use this phase to fix sources, then move to quarantine/reject when legitimate pass rates are high.',
    severity = 'info',
    actions = {
      'Aim for high pass rates on legitimate volume before p=quarantine.',
      'Document all sending platforms; none should be "surprise" sources in RUA.',
    },
  },
}

local function default_config()
  return {
    set_meta = true,
    set_guidance_meta = true,
    require_dmarc_report = false,
    min_count_for_issue = 1,
    top_sources_limit = 15,
    reporter_labels = {},
    known_sources = {},
    guidance = {},
    rua_domains = {}, -- optional: only process if recipient domain is listed
  }
end

local function merge_config_files(file_names)
  local target = default_config()
  for _, file_name in ipairs(file_names) do
    local data = utils.load_json_or_toml_file(file_name)
    for k, v in pairs(data) do
      if k == 'reporter_labels' or k == 'known_sources' or k == 'guidance' then
        target[k] = target[k] or {}
        utils.merge_into(v, target[k])
      elseif k == 'rua_domains' and type(v) == 'table' then
        -- allow list of domains or map
        if #v > 0 then
          for _, dom in ipairs(v) do
            target.rua_domains[dom] = true
          end
        else
          utils.merge_into(v, target.rua_domains)
        end
      else
        target[k] = v
      end
    end
  end
  return target
end

local function xml_unescape(s)
  if not s then
    return s
  end
  s = s:gsub('&lt;', '<'):gsub('&gt;', '>'):gsub('&quot;', '"'):gsub('&apos;', "'")
  s = s:gsub('&amp;', '&')
  return s
end

local function first_tag(xml, tag)
  -- non-greedy match for simple elements; handles optional attributes on open tag
  local pattern = string.format('<%s%%b></%s>', tag, tag)
  local block = xml:match(pattern)
  if block then
    return block
  end
  pattern = string.format('<%s>(.-)</%s>', tag, tag)
  return xml:match(pattern)
end

local function all_tags(xml, tag)
  local results = {}
  local pattern = string.format('<%s>(.-)</%s>', tag, tag)
  for inner in xml:gmatch(pattern) do
    table.insert(results, inner)
  end
  -- also with attributes on opening tag
  pattern = string.format('<%s%%s+[^>]*>(.-)</%s>', tag, tag)
  if #results == 0 then
    for inner in xml:gmatch(pattern) do
      table.insert(results, inner)
    end
  end
  return results
end

local function child_text(xml, tag)
  if not xml then
    return nil
  end
  local v = xml:match(string.format('<%s%%s*/>(.-)', tag)) -- unlikely
  v = xml:match(string.format('<%s>(.-)</%s>', tag, tag))
    or xml:match(string.format('<%s%%s+[^/]*/>(.-)', tag))
  if not v then
    v = xml:match(string.format('<%s[^>]*>(.-)</%s>', tag, tag))
  end
  if v then
    v = v:match '^%s*(.-)%s*$'
    if v:find '<' then
      -- nested; return full for further parse, else text only if simple
      local simple = v:match '^([^<]+)'
      if simple and not v:find('<' .. tag) then
        -- has children - caller may need block
      end
    end
    return xml_unescape(v:gsub('%s+', ' '):match '^%s*(.-)%s*$' or v)
  end
  return nil
end

local function child_text_simple(block, tag)
  if not block then
    return nil
  end
  local open = block:match(string.format('<%s[^>]*>(.-)</%s>', tag, tag))
  if not open then
    return nil
  end
  -- if still contains tags, take first text run only for leaf values
  if open:find '<' then
    local leaf = open:match '^%s*([^<]+)'
    if leaf and leaf:match '%S' then
      return xml_unescape(leaf:match '^%s*(.-)%s*$')
    end
    -- fully nested structure - return nil for "simple text" callers
    return nil
  end
  return xml_unescape(open:match '^%s*(.-)%s*$')
end

local function extract_feedback_xml(raw)
  if not raw or raw == '' then
    return nil
  end
  -- strip XML declaration noise
  local start_at = raw:find '<feedback'
  if not start_at then
    start_at = raw:find '<feedback>'
  end
  if not start_at then
    -- some reports use different root with namespace prefix
    start_at = raw:find '<[%w_:]*feedback[%s>]'
  end
  if not start_at then
    return nil
  end
  local xml = raw:sub(start_at)
  local end_at = xml:find '</feedback>'
  if end_at then
    xml = xml:sub(1, end_at + #'</feedback>' - 1)
  end
  return xml
end

function mod.extract_report_xml_from_message(msg)
  -- Try body parts: prefer text/xml application/xml, then any part containing <feedback
  local candidates = {}

  local function consider(data, mime)
    if not data or data == '' then
      return
    end
    local s = data
    if type(s) ~= 'string' then
      s = tostring(s)
    end
    if s:find '<feedback' or s:find '<feedback>' then
      table.insert(candidates, { priority = 1, data = s })
    elseif mime and (mime:find 'xml' or mime:find 'gzip') then
      table.insert(candidates, { priority = 3, data = s })
    end
  end

  -- Full message source often easiest
  local ok, source = pcall(function()
    return msg:get_data()
  end)
  if ok and source then
    consider(source, 'message/rfc822')
  end

  -- Walk MIME if available
  ok = pcall(function()
    local part = msg:get_mime_part and msg:get_mime_part()
    if not part then
      return
    end
    local function walk(p, depth)
      if depth > 20 then
        return
      end
      local ct = ''
      pcall(function()
        ct = tostring(p:content_type() or '')
      end)
      local body
      pcall(function()
        body = p:get_text() or p:get_data()
      end)
      if body then
        consider(body, ct)
      end
      local kids
      pcall(function()
        kids = p:children and p:children()
      end)
      if kids then
        for _, child in ipairs(kids) do
          walk(child, depth + 1)
        end
      end
    end
    walk(part, 0)
  end)

  table.sort(candidates, function(a, b)
    return a.priority < b.priority
  end)

  for _, c in ipairs(candidates) do
    local xml = extract_feedback_xml(c.data)
    if xml then
      return xml
    end
  end

  -- Last resort: search raw for embedded feedback
  if ok and source then
    return extract_feedback_xml(source)
  end
  return nil
end

function mod.parse_aggregate_report_xml(xml)
  xml = extract_feedback_xml(xml) or xml
  if not xml or not xml:find 'feedback' then
    return nil, 'not a DMARC aggregate feedback document'
  end

  local report_metadata = xml:match '<report_metadata>(.-)</report_metadata>' or ''
  local policy_published = xml:match '<policy_published>(.-)</policy_published>' or ''

  local meta = {
    org_name = child_text_simple(report_metadata, 'org_name'),
    email = child_text_simple(report_metadata, 'email'),
    extra_contact_info = child_text_simple(report_metadata, 'extra_contact_info'),
    report_id = child_text_simple(report_metadata, 'report_id'),
  }
  local date_range = report_metadata:match '<date_range>(.-)</date_range>' or ''
  meta.date_range = {
    begin_ts = tonumber(child_text_simple(date_range, 'begin')),
    end_ts = tonumber(child_text_simple(date_range, 'end')),
  }

  local policy = {
    domain = child_text_simple(policy_published, 'domain'),
    adkim = child_text_simple(policy_published, 'adkim'),
    aspf = child_text_simple(policy_published, 'aspf'),
    p = child_text_simple(policy_published, 'p'),
    sp = child_text_simple(policy_published, 'sp'),
    pct = child_text_simple(policy_published, 'pct'),
    fo = child_text_simple(policy_published, 'fo'),
  }

  local records = {}
  for record_inner in xml:gmatch '<record>(.-)</record>' do
    local row = record_inner:match '<row>(.-)</row>' or ''
    local identifiers = record_inner:match '<identifiers>(.-)</identifiers>' or ''
    local auth_results = record_inner:match '<auth_results>(.-)</auth_results>' or ''
    local policy_evaluated = row:match '<policy_evaluated>(.-)</policy_evaluated>' or ''

    local spf_auth = {}
    for spf_block in auth_results:gmatch '<spf>(.-)</spf>' do
      table.insert(spf_auth, {
        domain = child_text_simple(spf_block, 'domain'),
        scope = child_text_simple(spf_block, 'scope'),
        result = child_text_simple(spf_block, 'result'),
      })
    end

    local dkim_auth = {}
    for dkim_block in auth_results:gmatch '<dkim>(.-)</dkim>' do
      table.insert(dkim_auth, {
        domain = child_text_simple(dkim_block, 'domain'),
        selector = child_text_simple(dkim_block, 'selector'),
        result = child_text_simple(dkim_block, 'result'),
        human_result = child_text_simple(dkim_block, 'human_result'),
      })
    end

    local reasons = {}
    for reason_block in policy_evaluated:gmatch '<reason>(.-)</reason>' do
      table.insert(reasons, {
        type = child_text_simple(reason_block, 'type'),
        comment = child_text_simple(reason_block, 'comment'),
      })
    end

    table.insert(records, {
      source_ip = child_text_simple(row, 'source_ip'),
      count = tonumber(child_text_simple(row, 'count')) or 0,
      policy_evaluated = {
        disposition = child_text_simple(policy_evaluated, 'disposition'),
        dkim = child_text_simple(policy_evaluated, 'dkim'),
        spf = child_text_simple(policy_evaluated, 'spf'),
        reasons = reasons,
      },
      identifiers = {
        header_from = child_text_simple(identifiers, 'header_from'),
        envelope_from = child_text_simple(identifiers, 'envelope_from'),
        envelope_to = child_text_simple(identifiers, 'envelope_to'),
      },
      auth_results = {
        spf = spf_auth,
        dkim = dkim_auth,
      },
    })
  end

  return {
    version = child_text_simple(xml, 'version') or '1.0',
    report_metadata = meta,
    policy_published = policy,
    records = records,
  }
end

local function reporter_label(cfg, org_name, email)
  local labels = cfg.reporter_labels or {}
  if org_name and labels[org_name] then
    return labels[org_name]
  end
  if email then
    local domain = email:match '@([^>]+)$' or email:match '@(.+)$'
    if domain and labels[domain] then
      return labels[domain]
    end
    -- partial domain match
    for key, label in pairs(labels) do
      if domain and domain:find(key, 1, true) then
        return label
      end
      if org_name and org_name:lower():find(key:lower(), 1, true) then
        return label
      end
    end
  end
  if org_name and org_name:lower():find('google', 1, true) then
    return 'Gmail / Google'
  end
  if org_name and (org_name:lower():find('microsoft', 1, true) or org_name:lower():find('outlook', 1, true)) then
    return 'Microsoft Outlook / Hotmail'
  end
  if org_name and org_name:lower():find('yahoo', 1, true) then
    return 'Yahoo Mail'
  end
  return org_name or email or 'Unknown reporter'
end

local function ip_in_cidr(ip, cidr)
  -- minimal: exact match or prefix before /
  if ip == cidr then
    return true
  end
  local network = cidr:match '^([^/]+)/'
  if network and ip == network then
    return true
  end
  -- simple string prefix for documented /24 style labels only when full match fails
  if network and ip:sub(1, #network) == network then
    return true
  end
  return false
end

local function lookup_known_source(cfg, source_ip)
  if not source_ip or not cfg.known_sources then
    return nil
  end
  if cfg.known_sources[source_ip] then
    return cfg.known_sources[source_ip]
  end
  for key, info in pairs(cfg.known_sources) do
    if key:find '/' and ip_in_cidr(source_ip, key) then
      return info
    end
  end
  return nil
end

local function guidance_for(cfg, key)
  local g = (cfg.guidance and cfg.guidance[key]) or DEFAULT_GUIDANCE[key]
  if not g then
    return nil
  end
  return {
    key = key,
    title = g.title,
    customer_summary = g.customer_summary,
    severity = g.severity or 'info',
    actions = g.actions or {},
  }
end

local function classify_record(rec)
  local pe = rec.policy_evaluated or {}
  local dkim_eval = (pe.dkim or ''):lower()
  local spf_eval = (pe.spf or ''):lower()
  local disp = (pe.disposition or ''):lower()

  if dkim_eval == 'pass' or spf_eval == 'pass' then
    if dkim_eval == 'pass' and spf_eval ~= 'pass' then
      return 'spf_fail_dkim_pass', 'low'
    end
    if spf_eval == 'pass' and dkim_eval ~= 'pass' then
      return 'spf_pass_dkim_fail', 'medium'
    end
    return 'dmarc_pass', 'info'
  end

  if dkim_eval == 'fail' and spf_eval == 'fail' then
    return 'both_fail', 'high'
  end
  if spf_eval == 'fail' then
    return 'spf_fail_only_eval', 'medium'
  end
  if dkim_eval == 'fail' then
    return 'dkim_fail_only_eval', 'medium'
  end
  return 'both_fail', 'medium'
end

function mod.analyze_report(parsed, cfg)
  cfg = cfg or default_config()
  local min_count = cfg.min_count_for_issue or 1
  local top_n = cfg.top_sources_limit or 15

  local reporter = reporter_label(
    cfg,
    parsed.report_metadata and parsed.report_metadata.org_name,
    parsed.report_metadata and parsed.report_metadata.email
  )

  local totals = {
    messages = 0,
    pass = 0,
    fail = 0,
    disposition = { none = 0, quarantine = 0, reject = 0, other = 0 },
  }

  local by_issue = {}
  local source_rows = {}

  for _, rec in ipairs(parsed.records or {}) do
    local count = rec.count or 0
    totals.messages = totals.messages + count

    local pe = rec.policy_evaluated or {}
    local disp = (pe.disposition or 'none'):lower()
    if totals.disposition[disp] ~= nil then
      totals.disposition[disp] = totals.disposition[disp] + count
    else
      totals.disposition.other = totals.disposition.other + count
    end

    local issue_key, severity = classify_record(rec)
    if issue_key == 'dmarc_pass' then
      totals.pass = totals.pass + count
    else
      totals.fail = totals.fail + count
    end

    by_issue[issue_key] = by_issue[issue_key] or { count = 0, severity = severity, examples = {} }
    by_issue[issue_key].count = by_issue[issue_key].count + count

    if count >= min_count then
      local known = lookup_known_source(cfg, rec.source_ip)
      table.insert(source_rows, {
        source_ip = rec.source_ip,
        count = count,
        header_from = rec.identifiers and rec.identifiers.header_from,
        disposition = pe.disposition,
        dkim_eval = pe.dkim,
        spf_eval = pe.spf,
        issue_key = issue_key,
        severity = severity,
        known_source = known and (known.label or known.name) or nil,
        expected = known and known.expected or false,
        auth_results = rec.auth_results,
      })
    end
  end

  table.sort(source_rows, function(a, b)
    return (a.count or 0) > (b.count or 0)
  end)

  local top_sources = {}
  for i = 1, math.min(top_n, #source_rows) do
    table.insert(top_sources, source_rows[i])
  end

  -- Build ordered guidance list for customer communication
  local guidance_items = {}
  local seen_keys = {}

  local policy_p = parsed.policy_published and parsed.policy_published.p
  if policy_p and policy_p:lower() == 'none' then
    local g = guidance_for(cfg, 'policy_none_monitoring')
    if g then
      table.insert(guidance_items, g)
      seen_keys['policy_none_monitoring'] = true
    end
  end

  if totals.disposition.reject > 0 then
    local g = guidance_for(cfg, 'disposition_reject')
    if g then
      g.count = totals.disposition.reject
      table.insert(guidance_items, g)
      seen_keys['disposition_reject'] = true
    end
  elseif totals.disposition.quarantine > 0 then
    local g = guidance_for(cfg, 'disposition_quarantine')
    if g then
      g.count = totals.disposition.quarantine
      table.insert(guidance_items, g)
      seen_keys['disposition_quarantine'] = true
    end
  end

  -- Issue types sorted by volume
  local issue_list = {}
  for key, info in pairs(by_issue) do
    if key ~= 'dmarc_pass' and info.count >= min_count then
      table.insert(issue_list, { key = key, count = info.count, severity = info.severity })
    end
  end
  table.sort(issue_list, function(a, b)
    return a.count > b.count
  end)

  for _, item in ipairs(issue_list) do
    if not seen_keys[item.key] then
      local g = guidance_for(cfg, item.key)
      if g then
        g.count = item.count
        table.insert(guidance_items, g)
        seen_keys[item.key] = true
      end
    end
  end

  -- Unknown sources among top failures
  for _, row in ipairs(top_sources) do
    if row.issue_key ~= 'dmarc_pass' and not row.expected and not row.known_source then
      if not seen_keys['unknown_source'] then
        local g = guidance_for(cfg, 'unknown_source')
        if g then
          g.example_ip = row.source_ip
          g.example_count = row.count
          table.insert(guidance_items, g)
          seen_keys['unknown_source'] = true
        end
      end
      break
    end
  end

  local pass_rate = 0
  if totals.messages > 0 then
    pass_rate = math.floor((totals.pass / totals.messages) * 1000 + 0.5) / 10
  end

  local executive_summary = string.format(
    '%s reported %d messages for domain %s in this period: ~%.1f%% DMARC pass (%d pass / %d fail eval). Policy p=%s.',
    reporter,
    totals.messages,
    (parsed.policy_published and parsed.policy_published.domain) or 'unknown',
    pass_rate,
    totals.pass,
    totals.fail,
    policy_p or 'unknown'
  )

  if totals.disposition.reject > 0 then
    executive_summary = executive_summary
      .. string.format(' %d messages saw reject disposition.', totals.disposition.reject)
  elseif totals.disposition.quarantine > 0 then
    executive_summary = executive_summary
      .. string.format(' %d messages saw quarantine disposition.', totals.disposition.quarantine)
  end

  return {
    is_dmarc_report = true,
    reporter = reporter,
    reporter_org = parsed.report_metadata and parsed.report_metadata.org_name,
    reporter_email = parsed.report_metadata and parsed.report_metadata.email,
    report_id = parsed.report_metadata and parsed.report_metadata.report_id,
    date_range = parsed.report_metadata and parsed.report_metadata.date_range,
    policy_published = parsed.policy_published,
    totals = totals,
    pass_rate_percent = pass_rate,
    by_issue = by_issue,
    top_sources = top_sources,
    guidance = guidance_items,
    executive_summary = executive_summary,
    customer_briefing = mod.format_customer_briefing {
      reporter = reporter,
      executive_summary = executive_summary,
      guidance = guidance_items,
      top_sources = top_sources,
      policy_published = parsed.policy_published,
      pass_rate_percent = pass_rate,
    },
  }
end

function mod.format_customer_briefing(analysis)
  local lines = {}
  table.insert(lines, '## DMARC aggregate report briefing')
  table.insert(lines, '')
  table.insert(lines, '**Reporter (ISP/receiver):** ' .. (analysis.reporter or 'unknown'))
  if analysis.policy_published and analysis.policy_published.domain then
    table.insert(lines, '**Domain in report:** ' .. analysis.policy_published.domain)
  end
  table.insert(lines, '')
  table.insert(lines, analysis.executive_summary or '')
  table.insert(lines, '')
  table.insert(lines, '### What this means for you')
  table.insert(lines, '')
  for i, g in ipairs(analysis.guidance or {}) do
    table.insert(
      lines,
      string.format(
        '%d. **%s** (%s)%s',
        i,
        g.title or g.key,
        g.severity or 'info',
        g.count and string.format(' — affecting ~%d messages in this report', g.count) or ''
      )
    )
    table.insert(lines, '   ' .. (g.customer_summary or ''))
    if g.actions and #g.actions > 0 then
      table.insert(lines, '   Recommended actions:')
      for _, act in ipairs(g.actions) do
        table.insert(lines, '   - ' .. act)
      end
    end
    table.insert(lines, '')
  end

  if analysis.top_sources and #analysis.top_sources > 0 then
    table.insert(lines, '### Top sources in this report')
    table.insert(lines, '')
    table.insert(
      lines,
      '| source_ip | count | header_from | spf | dkim | disposition | issue |'
    )
    table.insert(lines, '|---|---:|---|---|---|---|---|')
    for _, row in ipairs(analysis.top_sources) do
      table.insert(
        lines,
        string.format(
          '| %s | %d | %s | %s | %s | %s | %s |',
          row.source_ip or '',
          row.count or 0,
          row.header_from or '',
          row.spf_eval or '',
          row.dkim_eval or '',
          row.disposition or '',
          row.issue_key or ''
        )
      )
    end
    table.insert(lines, '')
  end

  table.insert(
    lines,
    '_Generated by KumoMTA policy-extras.dmarc_rua from a receiver aggregate (RUA) report. Fix legitimate sources before tightening DMARC policy._'
  )

  return table.concat(lines, '\n')
end

function mod.analyze_xml(xml, cfg)
  local parsed, err = mod.parse_aggregate_report_xml(xml)
  if not parsed then
    return nil, err
  end
  return mod.analyze_report(parsed, cfg), nil
end

function mod.process_message(msg, cfg)
  cfg = cfg or (mod.CONFIGURED and mod.CONFIGURED.get_data()) or default_config()

  if cfg.rua_domains and next(cfg.rua_domains) then
    local ok, recip = pcall(function()
      return msg:recipient()
    end)
    if ok and recip then
      local domain = recip.domain
      if domain and not cfg.rua_domains[domain] and not cfg.rua_domains[tostring(domain)] then
        return nil
      end
    end
  end

  local xml = mod.extract_report_xml_from_message(msg)
  if not xml then
    if cfg.require_dmarc_report then
      return {
        is_dmarc_report = false,
        error = 'message did not contain a DMARC aggregate <feedback> report',
      }
    end
    return nil
  end

  local analysis, err = mod.analyze_xml(xml, cfg)
  if not analysis then
    return {
      is_dmarc_report = false,
      error = err or 'failed to parse DMARC report',
    }
  end

  if cfg.set_meta then
    pcall(function()
      msg:set_meta('dmarc_rua_analysis', analysis)
    end)
    pcall(function()
      msg:set_meta('dmarc_rua_reporter', analysis.reporter)
    end)
    pcall(function()
      msg:set_meta('dmarc_rua_pass_rate', analysis.pass_rate_percent)
    end)
  end
  if cfg.set_guidance_meta then
    pcall(function()
      msg:set_meta('dmarc_rua_guidance', analysis.customer_briefing)
    end)
    pcall(function()
      msg:set_meta('dmarc_rua_executive_summary', analysis.executive_summary)
    end)
  end

  return analysis
end

function mod:setup(data_files)
  if mod.CONFIGURED then
    error 'dmarc_rua module has already been configured'
  end

  if type(data_files) == 'string' then
    data_files = { data_files }
  end
  if not data_files or #data_files == 0 then
    data_files = { DEFAULT_SHIPPED }
  end

  local cached_load = kumo.memoize(merge_config_files, {
    name = 'dmarc_rua_data',
    ttl = '5 minutes',
    capacity = 10,
    invalidate_with_epoch = true,
  })

  local function get_data()
    return cached_load(data_files)
  end

  local function process_message(msg)
    return mod.process_message(msg, get_data())
  end

  mod.CONFIGURED = {
    data_files = data_files,
    get_data = get_data,
    process_message = process_message,
    analyze_xml = function(xml)
      return mod.analyze_xml(xml, get_data())
    end,
    parse_xml = mod.parse_aggregate_report_xml,
  }

  return mod.CONFIGURED
end

function mod:test()
  local sample = [=[<?xml version="1.0"?>
<feedback>
  <version>1.0</version>
  <report_metadata>
    <org_name>google.com</org_name>
    <email>noreply-dmarc-support@google.com</email>
    <report_id>1234567890</report_id>
    <date_range><begin>1717200000</begin><end>1717286400</end></date_range>
  </report_metadata>
  <policy_published>
    <domain>customer.example</domain>
    <adkim>r</adkim>
    <aspf>r</aspf>
    <p>none</p>
    <sp>none</sp>
    <pct>100</pct>
  </policy_published>
  <record>
    <row>
      <source_ip>203.0.113.10</source_ip>
      <count>100</count>
      <policy_evaluated>
        <disposition>none</disposition>
        <dkim>pass</dkim>
        <spf>pass</spf>
      </policy_evaluated>
    </row>
    <identifiers><header_from>customer.example</header_from></identifiers>
    <auth_results>
      <dkim><domain>customer.example</domain><result>pass</result><selector>s1</selector></dkim>
      <spf><domain>customer.example</domain><scope>mfrom</scope><result>pass</result></spf>
    </auth_results>
  </record>
  <record>
    <row>
      <source_ip>198.51.100.50</source_ip>
      <count>40</count>
      <policy_evaluated>
        <disposition>none</disposition>
        <dkim>fail</dkim>
        <spf>fail</spf>
      </policy_evaluated>
    </row>
    <identifiers><header_from>customer.example</header_from></identifiers>
    <auth_results>
      <spf><domain>customer.example</domain><scope>mfrom</scope><result>fail</result></spf>
    </auth_results>
  </record>
  <record>
    <row>
      <source_ip>192.0.2.9</source_ip>
      <count>5</count>
      <policy_evaluated>
        <disposition>quarantine</disposition>
        <dkim>fail</dkim>
        <spf>pass</spf>
      </policy_evaluated>
    </row>
    <identifiers><header_from>customer.example</header_from></identifiers>
    <auth_results>
      <spf><domain>customer.example</domain><scope>mfrom</scope><result>pass</result></spf>
    </auth_results>
  </record>
</feedback>]=]

  local parsed, err = mod.parse_aggregate_report_xml(sample)
  utils.assert_eq(err, nil)
  utils.assert_eq(parsed.report_metadata.org_name, 'google.com')
  utils.assert_eq(parsed.policy_published.domain, 'customer.example')
  utils.assert_eq(#parsed.records, 3)
  utils.assert_eq(parsed.records[1].source_ip, '203.0.113.10')
  utils.assert_eq(parsed.records[1].count, 100)

  local cfg = merge_config_files {
    kumo.serde.toml_parse [[
min_count_for_issue = 1
top_sources_limit = 10

[reporter_labels]
"google.com" = "Gmail / Google"

[known_sources."203.0.113.10"]
label = "Primary MTA"
expected = true
]],
  }

  local analysis = mod.analyze_report(parsed, cfg)
  utils.assert_eq(analysis.is_dmarc_report, true)
  utils.assert_eq(analysis.reporter, 'Gmail / Google')
  utils.assert_eq(analysis.totals.messages, 145)
  utils.assert_eq(analysis.totals.pass, 100)
  utils.assert_eq(analysis.totals.fail, 45)
  utils.assert_eq(analysis.totals.disposition.quarantine, 5)

  local has_both_fail = false
  local has_policy_none = false
  for _, g in ipairs(analysis.guidance) do
    if g.key == 'both_fail' then
      has_both_fail = true
    end
    if g.key == 'policy_none_monitoring' then
      has_policy_none = true
    end
  end
  utils.assert_eq(has_both_fail, true)
  utils.assert_eq(has_policy_none, true)
  utils.assert_eq(type(analysis.customer_briefing), 'string')
  utils.assert_eq(analysis.customer_briefing:find('Gmail / Google', 1, true) ~= nil, true)

  -- analyze_xml convenience
  local a2, e2 = mod.analyze_xml(sample, cfg)
  utils.assert_eq(e2, nil)
  utils.assert_eq(a2.totals.messages, 145)

  print 'dmarc_rua.lua tests passed'
end

return mod
