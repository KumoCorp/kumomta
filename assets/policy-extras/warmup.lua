local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

--[[
Automatic IP warmup helper.

Enforces gradual overall per-source volume ramps by injecting
`additional_source_selection_rates` into egress path config based on
`warmup_start` + a named schedule preset (or custom schedule).

Usage:

Create `/opt/kumomta/etc/warmup.toml` that layers on shipped presets:

```toml
# Optional: override default preset name
default_schedule = "conservative"

[source."ip-3"]
warmup_start = "2026-06-01"
schedule = "conservative"
```

Then in init.lua, compose with shaping (recommended):

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

Or apply manually:

```lua
kumo.on('get_egress_path_config', function(domain, source, site)
  local params = shaper.get_egress_path_config(domain, source, site, true)
  warmer.apply_to_params(params, source)
  return kumo.make_egress_path(params)
end)
```

See docs/userguide/configuration/ip_warmup.md for operator guidance.
]]

local DEFAULT_SHIPPED = '/opt/kumomta/share/policy-extras/warmup.toml'

local function parse_ymd_components(date_str)
  if type(date_str) ~= 'string' then
    return nil, nil, nil, 'warmup_start must be a string YYYY-MM-DD'
  end
  local y, m, d = date_str:match '^(%d%d%d%d)%-(%d%d)%-(%d%d)$'
  if not y then
    return nil,
      nil,
      nil,
      string.format('invalid warmup_start %q; expected YYYY-MM-DD', date_str)
  end
  return tonumber(y), tonumber(m), tonumber(d), nil
end

-- Unix timestamp at UTC midnight for a calendar date (via kumo.time when available).
local function utc_midnight_for_ymd(y, m, d)
  if kumo.time and kumo.time.with_ymd_hms then
    local t = kumo.time.with_ymd_hms(y, m, d, 0, 0, 0)
    return t.unix_timestamp
  end
  -- Fallback for environments without kumo.time (less accurate off-UTC hosts)
  return os.time { year = y, month = m, day = d, hour = 0, min = 0, sec = 0, isdst = false }
end

local function utc_midnight_for_timestamp(ts)
  if kumo.time and kumo.time.from_unix_timestamp then
    local t = kumo.time.from_unix_timestamp(ts)
    -- RFC3339 is UTC; parse Y-M-D from it
    local y, m, d = t.rfc3339:match '^(%d%d%d%d)%-(%d%d)%-(%d%d)'
    if y then
      return utc_midnight_for_ymd(tonumber(y), tonumber(m), tonumber(d))
    end
  end
  local ud = os.date('!*t', ts)
  return utc_midnight_for_ymd(ud.year, ud.month, ud.day)
end

local function parse_ymd(date_str)
  local y, m, d, err = parse_ymd_components(date_str)
  if err then
    return nil, err
  end
  local ok, ts = pcall(utc_midnight_for_ymd, y, m, d)
  if not ok or not ts then
    return nil, string.format('invalid calendar date in warmup_start %q', date_str)
  end
  return ts
end

-- Exported for tests: compute 1-based warmup day index.
function mod.compute_day_index(warmup_start, now_ts, day_offset, before_start_behavior)
  local y, m, d, err = parse_ymd_components(warmup_start)
  if err then
    return nil, err
  end

  local start_day = utc_midnight_for_ymd(y, m, d)
  now_ts = now_ts or os.time()
  day_offset = day_offset or 0

  local now_day = utc_midnight_for_timestamp(now_ts)
  local days = math.floor((now_day - start_day) / 86400) + 1 + day_offset

  if days < 1 then
    if before_start_behavior == 'block' then
      return 0 -- special: blocked / not started
    end
    return 1
  end
  return days
end

local function normalize_limit_entry(value, max_burst)
  if value == nil then
    return nil
  end
  if type(value) == 'number' then
    if value < 0 or value ~= math.floor(value) then
      return nil, string.format('schedule limit must be a non-negative integer, got %s', value)
    end
    return {
      daily = value,
      throttle = string.format('%d/day,max_burst=%d', value, max_burst or 1),
    }
  end
  if type(value) == 'string' then
    -- Full throttle spec passthrough
    return {
      daily = nil,
      throttle = value,
    }
  end
  if type(value) == 'table' then
    local daily = value.daily or value.limit or value.day
    local throttle = value.throttle or value.rate
    if throttle then
      return { daily = daily, throttle = throttle, hourly = value.hourly }
    end
    if daily ~= nil then
      local n = tonumber(daily)
      if not n then
        return nil, 'schedule entry daily/limit must be a number or throttle string'
      end
      return {
        daily = n,
        throttle = string.format('%d/day,max_burst=%d', n, max_burst or 1),
        hourly = value.hourly,
      }
    end
  end
  return nil, string.format('unsupported schedule entry type %s', type(value))
end

local function schedule_max_day(schedule)
  local max_d = 0
  for k, _ in pairs(schedule) do
    local n = tonumber(k)
    if n and n > max_d then
      max_d = n
    end
  end
  return max_d
end

local function schedule_entry_for_day(schedule, day)
  if not schedule then
    return nil
  end
  -- TOML may key as string or number depending on loader
  return schedule[day] or schedule[tostring(day)] or schedule[string.format('%d', day)]
end

local function merge_warmup_files(file_names)
  local target = {
    default_schedule = 'conservative',
    timezone = 'UTC',
    active_sending_hours = 12,
    max_burst = 1,
    apply_hourly_spread = true,
    hold_final_day = false,
    before_start_behavior = 'day1',
    schedules = {},
    sources = {},
  }

  for _, file_name in ipairs(file_names) do
    local data = utils.load_json_or_toml_file(file_name)
    if data.default_schedule ~= nil then
      target.default_schedule = data.default_schedule
    end
    if data.timezone ~= nil then
      target.timezone = data.timezone
    end
    if data.active_sending_hours ~= nil then
      target.active_sending_hours = data.active_sending_hours
    end
    if data.max_burst ~= nil then
      target.max_burst = data.max_burst
    end
    if data.apply_hourly_spread ~= nil then
      target.apply_hourly_spread = data.apply_hourly_spread
    end
    if data.hold_final_day ~= nil then
      target.hold_final_day = data.hold_final_day
    end
    if data.before_start_behavior ~= nil then
      target.before_start_behavior = data.before_start_behavior
    end

    -- [schedule.name] blocks
    if data.schedule then
      for sched_name, sched_def in pairs(data.schedule) do
        target.schedules[sched_name] = target.schedules[sched_name] or {}
        for day_key, limit in pairs(sched_def) do
          target.schedules[sched_name][day_key] = limit
        end
      end
    end

    -- [source."name"] blocks
    if data.source then
      for source_name, src_def in pairs(data.source) do
        target.sources[source_name] = target.sources[source_name] or {}
        utils.merge_into(src_def, target.sources[source_name])
      end
    end
  end

  return target
end

local function effective_bool(src_val, global_val)
  if src_val ~= nil then
    return src_val
  end
  return global_val
end

local function ensure_selection_rates(params)
  if not params.additional_source_selection_rates then
    params.additional_source_selection_rates = {}
  end
  return params.additional_source_selection_rates
end

local function set_warmup_rates(params, source_name, daily_throttle, hourly_throttle, extra)
  local rates = ensure_selection_rates(params)
  rates['warmup-source-' .. source_name] = daily_throttle
  if hourly_throttle then
    rates['warmup-source-' .. source_name .. '-hourly'] = hourly_throttle
  end
  if extra then
    for k, v in pairs(extra) do
      rates[k] = v
    end
  end
end

-- Resolve effective warmup limits for a source. Used by apply_to_params and tests.
function mod.resolve_source_warmup(cfg, source_name, now_ts)
  local src = cfg.sources[source_name]
  if not src then
    return { active = false, reason = 'not_enrolled' }
  end

  local status = src.status or 'active'
  if status == 'complete' then
    if src.post_warmup_rate then
      return {
        active = true,
        reason = 'post_warmup',
        daily_throttle = src.post_warmup_rate,
        day = nil,
      }
    end
    return { active = false, reason = 'complete' }
  end

  if status == 'paused' then
    return {
      active = true,
      reason = 'paused',
      daily_throttle = src.paused_rate or '0/day,max_burst=1',
      day = nil,
    }
  end

  if not src.warmup_start then
    return { active = false, reason = 'missing_warmup_start', error = true }
  end

  local schedule_name = src.schedule or cfg.default_schedule
  local schedule = cfg.schedules[schedule_name]
  if not schedule then
    return {
      active = false,
      reason = 'unknown_schedule',
      error = true,
      schedule_name = schedule_name,
    }
  end

  local before = src.before_start_behavior or cfg.before_start_behavior or 'day1'
  local day, day_err = mod.compute_day_index(
    src.warmup_start,
    now_ts,
    src.day_offset,
    before
  )
  if not day then
    return { active = false, reason = 'bad_warmup_start', error = true, detail = day_err }
  end

  if day == 0 then
    return {
      active = true,
      reason = 'before_start_block',
      day = 0,
      daily_throttle = '0/day,max_burst=1',
    }
  end

  local hold_final = effective_bool(src.hold_final_day, cfg.hold_final_day)
  local max_day = schedule_max_day(schedule)
  local entry_raw = schedule_entry_for_day(schedule, day)

  if not entry_raw then
    if hold_final and max_day > 0 then
      entry_raw = schedule_entry_for_day(schedule, max_day)
      day = max_day
    elseif src.post_warmup_rate then
      return {
        active = true,
        reason = 'post_warmup',
        day = day,
        daily_throttle = src.post_warmup_rate,
      }
    else
      return { active = false, reason = 'schedule_complete', day = day }
    end
  end

  local max_burst = src.max_burst or cfg.max_burst or 1
  local entry, norm_err = normalize_limit_entry(entry_raw, max_burst)
  if not entry then
    return {
      active = false,
      reason = 'bad_schedule_entry',
      error = true,
      detail = norm_err,
      day = day,
    }
  end

  local apply_hourly = effective_bool(src.apply_hourly_spread, cfg.apply_hourly_spread)
  local hourly_throttle = nil
  if entry.hourly then
    if type(entry.hourly) == 'number' then
      hourly_throttle =
        string.format('%d/hour,max_burst=%d', entry.hourly, max_burst)
    else
      hourly_throttle = entry.hourly
    end
  elseif apply_hourly and entry.daily and entry.daily > 0 then
    local hours = src.active_sending_hours or cfg.active_sending_hours or 12
    if hours < 1 then
      hours = 1
    end
    local per_hour = math.max(1, math.ceil(entry.daily / hours))
    hourly_throttle = string.format('%d/hour,max_burst=%d', per_hour, max_burst)
  end

  return {
    active = true,
    reason = 'warming',
    day = day,
    schedule_name = schedule_name,
    daily_throttle = entry.throttle,
    hourly_throttle = hourly_throttle,
    extra_selection_rates = src.extra_selection_rates,
  }
end

function mod.apply_to_params(params, egress_source, now_ts)
  if not mod.CONFIGURED then
    return params
  end
  local cfg = mod.CONFIGURED.get_data()
  local resolved = mod.resolve_source_warmup(cfg, egress_source, now_ts)
  if not resolved.active then
    return params
  end

  set_warmup_rates(
    params,
    egress_source,
    resolved.daily_throttle,
    resolved.hourly_throttle,
    resolved.extra_selection_rates
  )
  return params
end

function mod:setup(data_files)
  if mod.CONFIGURED then
    error 'warmup module has already been configured'
  end

  if type(data_files) == 'string' then
    data_files = { data_files }
  end
  if not data_files or #data_files == 0 then
    data_files = { DEFAULT_SHIPPED }
  end

  local cached_load_data = kumo.memoize(merge_warmup_files, {
    name = 'warmup_data',
    ttl = '5 minutes',
    capacity = 10,
    invalidate_with_epoch = true,
  })

  local function get_data()
    return cached_load_data(data_files)
  end

  local function apply_to_params(params, egress_source, now_ts)
    local cfg = get_data()
    local resolved = mod.resolve_source_warmup(cfg, egress_source, now_ts)
    if not resolved.active then
      return params
    end
    set_warmup_rates(
      params,
      egress_source,
      resolved.daily_throttle,
      resolved.hourly_throttle,
      resolved.extra_selection_rates
    )
    return params
  end

  local function wrap(inner_get_egress_path_config)
    return function(domain, source, site, skip_make)
      local params
      if inner_get_egress_path_config then
        params = inner_get_egress_path_config(domain, source, site, true)
      else
        params = {}
      end
      apply_to_params(params, source)
      if skip_make then
        return params
      end
      return kumo.make_egress_path(params)
    end
  end

  mod.CONFIGURED = {
    data_files = data_files,
    get_data = get_data,
    apply_to_params = apply_to_params,
    wrap = wrap,
    resolve = function(source_name, now_ts)
      return mod.resolve_source_warmup(get_data(), source_name, now_ts)
    end,
  }

  return mod.CONFIGURED
end

kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local data = mod.CONFIGURED.get_data()
  local failed = false

  local function show_context()
    if failed then
      return
    end
    failed = true
    print 'Issues found in the combined set of warmup files:'
    for _, file_name in ipairs(mod.CONFIGURED.data_files) do
      if type(file_name) == 'table' then
        print ' - (inline table)'
      else
        print(string.format(' - %s', file_name))
      end
    end
  end

  if data.timezone and data.timezone ~= 'UTC' then
    show_context()
    print(
      string.format(
        'WARNING: timezone=%q is configured but day boundaries currently use UTC. Set timezone="UTC" or expect day rolls at UTC midnight.',
        data.timezone
      )
    )
  end

  for sched_name, schedule in pairs(data.schedules) do
    if schedule_max_day(schedule) < 1 then
      show_context()
      print(string.format('schedule %q has no day entries', sched_name))
      kumo.validation_failed()
    end
    for day_key, limit in pairs(schedule) do
      local entry, err = normalize_limit_entry(limit, data.max_burst)
      if not entry then
        show_context()
        print(
          string.format(
            'schedule %q day %s: %s',
            sched_name,
            tostring(day_key),
            err or 'invalid'
          )
        )
        kumo.validation_failed()
      end
    end
  end

  for source_name, src in pairs(data.sources) do
    local status = src.status or 'active'
    if status ~= 'active' and status ~= 'complete' and status ~= 'paused' then
      show_context()
      print(
        string.format(
          'source %q: invalid status %q (expected active|complete|paused)',
          source_name,
          tostring(status)
        )
      )
      kumo.validation_failed()
    end

    if status == 'active' then
      if not src.warmup_start then
        show_context()
        print(string.format('source %q: warmup_start is required when status is active', source_name))
        kumo.validation_failed()
      else
        local _ts, err = parse_ymd(src.warmup_start)
        if err then
          show_context()
          print(string.format('source %q: %s', source_name, err))
          kumo.validation_failed()
        end
      end

      local schedule_name = src.schedule or data.default_schedule
      if not data.schedules[schedule_name] then
        show_context()
        print(
          string.format(
            'source %q: unknown schedule %q',
            source_name,
            tostring(schedule_name)
          )
        )
        kumo.validation_failed()
      end
    end

    local resolved = mod.resolve_source_warmup(data, source_name)
    if resolved.error then
      show_context()
      print(
        string.format(
          'source %q: %s %s',
          source_name,
          resolved.reason,
          resolved.detail or resolved.schedule_name or ''
        )
      )
      kumo.validation_failed()
    end
  end
end)

function mod:test()
  -- Day index: start date is day 1
  local start = '2026-06-01'
  local day1_noon = os.time { year = 2026, month = 6, day = 1, hour = 12, min = 0, sec = 0 }
  local day2_noon = os.time { year = 2026, month = 6, day = 2, hour = 12, min = 0, sec = 0 }
  local before = os.time { year = 2026, month = 5, day = 31, hour = 12, min = 0, sec = 0 }

  utils.assert_eq(mod.compute_day_index(start, day1_noon, 0, 'day1'), 1)
  utils.assert_eq(mod.compute_day_index(start, day2_noon, 0, 'day1'), 2)
  utils.assert_eq(mod.compute_day_index(start, before, 0, 'day1'), 1)
  utils.assert_eq(mod.compute_day_index(start, before, 0, 'block'), 0)
  utils.assert_eq(mod.compute_day_index(start, day1_noon, 2, 'day1'), 3)

  local cfg = merge_warmup_files {
    kumo.serde.toml_parse [[
default_schedule = "conservative"
max_burst = 1
apply_hourly_spread = true
active_sending_hours = 10
hold_final_day = false

[schedule.conservative]
1 = 50
2 = 100
3 = 200

[schedule.custom]
1 = 10
2 = "25/day,max_burst=2"

[source."ip-new"]
warmup_start = "2026-06-01"
schedule = "conservative"

[source."ip-done"]
warmup_start = "2026-06-01"
status = "complete"

[source."ip-paused"]
warmup_start = "2026-06-01"
status = "paused"

[source."ip-hold"]
warmup_start = "2026-06-01"
schedule = "conservative"
hold_final_day = true

[source."ip-post"]
warmup_start = "2026-06-01"
schedule = "conservative"
post_warmup_rate = "999/day,max_burst=1"

[source."ip-custom"]
warmup_start = "2026-06-01"
schedule = "custom"
]],
  }

  -- Not enrolled
  local r = mod.resolve_source_warmup(cfg, 'unknown', day1_noon)
  utils.assert_eq(r.active, false)
  utils.assert_eq(r.reason, 'not_enrolled')

  -- Day 1 warming with hourly spread
  r = mod.resolve_source_warmup(cfg, 'ip-new', day1_noon)
  utils.assert_eq(r.active, true)
  utils.assert_eq(r.reason, 'warming')
  utils.assert_eq(r.day, 1)
  utils.assert_eq(r.daily_throttle, '50/day,max_burst=1')
  utils.assert_eq(r.hourly_throttle, '5/hour,max_burst=1') -- ceil(50/10)

  -- Day 2
  r = mod.resolve_source_warmup(cfg, 'ip-new', day2_noon)
  utils.assert_eq(r.day, 2)
  utils.assert_eq(r.daily_throttle, '100/day,max_burst=1')

  -- Past schedule without hold => complete
  local day10 = os.time { year = 2026, month = 6, day = 10, hour = 12, min = 0, sec = 0 }
  r = mod.resolve_source_warmup(cfg, 'ip-new', day10)
  utils.assert_eq(r.active, false)
  utils.assert_eq(r.reason, 'schedule_complete')

  -- hold_final_day keeps last day rate
  r = mod.resolve_source_warmup(cfg, 'ip-hold', day10)
  utils.assert_eq(r.active, true)
  utils.assert_eq(r.daily_throttle, '200/day,max_burst=1')

  -- post_warmup_rate after schedule
  r = mod.resolve_source_warmup(cfg, 'ip-post', day10)
  utils.assert_eq(r.active, true)
  utils.assert_eq(r.reason, 'post_warmup')
  utils.assert_eq(r.daily_throttle, '999/day,max_burst=1')

  -- complete status
  r = mod.resolve_source_warmup(cfg, 'ip-done', day1_noon)
  utils.assert_eq(r.active, false)
  utils.assert_eq(r.reason, 'complete')

  -- paused
  r = mod.resolve_source_warmup(cfg, 'ip-paused', day1_noon)
  utils.assert_eq(r.active, true)
  utils.assert_eq(r.reason, 'paused')
  utils.assert_eq(r.daily_throttle, '0/day,max_burst=1')

  -- custom string throttle on day 2
  r = mod.resolve_source_warmup(cfg, 'ip-custom', day2_noon)
  utils.assert_eq(r.daily_throttle, '25/day,max_burst=2')

  -- apply_to_params with setup
  mod.CONFIGURED = nil
  local warmer = mod:setup {
    kumo.serde.toml_parse [[
[schedule.only]
1 = 50

[source."s1"]
warmup_start = "2026-06-01"
schedule = "only"
apply_hourly_spread = false
]],
  }

  local params = { connection_limit = 10 }
  warmer.apply_to_params(params, 's1', day1_noon)
  utils.assert_eq(params.connection_limit, 10)
  utils.assert_eq(params.additional_source_selection_rates['warmup-source-s1'], '50/day,max_burst=1')
  utils.assert_eq(params.additional_source_selection_rates['warmup-source-s1-hourly'], nil)

  -- unenrolled source unchanged
  local params2 = {}
  warmer.apply_to_params(params2, 'other', day1_noon)
  utils.assert_eq(params2.additional_source_selection_rates, nil)

  -- wrap passes through skip_make
  local inner_called = false
  local wrapped = warmer.wrap(function(domain, source, site, skip_make)
    inner_called = true
    utils.assert_eq(skip_make, true)
    return { enable_tls = 'Opportunistic' }
  end)
  local out = wrapped('example.com', 's1', 'site', true)
  utils.assert_eq(inner_called, true)
  utils.assert_eq(out.enable_tls, 'Opportunistic')
  utils.assert_eq(out.additional_source_selection_rates['warmup-source-s1'], '50/day,max_burst=1')

  mod.CONFIGURED = nil
  print 'warmup.lua tests passed'
end

return mod
