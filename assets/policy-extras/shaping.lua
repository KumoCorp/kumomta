local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

--[[
Usage:

Create a `/opt/kumomta/etc/shaping.json` with contents like:

```json
{
  "gmail.com": {
    // "mx_rollup": false, // if false, is NOT shared with other domains with same mx
    "connection_limit": 3,

    // Entries that are themselves objects are considered to be
    // source configuration. These will take precendence over
    // the more general configuration for the domain/site
    "sources": {
      "source-0": {
        "connection_limit": 5
      }
    }
  }
}
```

---
local shaping = require 'policy-extras.shaping'
kumo.on('get_egress_path_config', shaping:setup('/opt/kumomta/etc/shaping.json'))
---

Alternatively, you could use a TOML file instead.
]]

function mod:setup(extra_files)
  local function load_shaping_data(file_names)
    local site_to_domains = {}
    local result = {
      by_site = {},
      by_domain = {},
    }
    for _, filename in ipairs(file_names) do
      local data = utils.load_json_or_toml_file(filename)
      -- print('Loaded', kumo.json_encode_pretty(data))
      for domain, config in pairs(data) do
        local entry = {
          sources = {},
          params = {},
        }

        local mx_rollup = true
        if config.mx_rollup ~= nil then
          mx_rollup = config.mx_rollup
        end
        local replace_base = false
        if config.replace_base ~= nil then
          replace_base = config.replace_base
        end

        config.mx_rollup = nil
        config.replace_base = nil

        if domain == 'default' then
          mx_rollup = false
        end

        for k, v in pairs(config) do
          if k == 'sources' then
            entry.sources = v
          else
            entry.params[k] = v
          end
        end

        if mx_rollup then
          local site_name = kumo.dns.lookup_mx(domain).site_name

          if site_name == '' then
            error(
              string.format(
                'domain %s has a NULL MX and cannot be used with mx_rollup=true',
                domain
              )
            )
          end

          if not replace_base then
            utils.merge_into(result.by_site[site_name], entry)
          end

          result.by_site[site_name] = entry

          local site_domains = site_to_domains[site_name] or {}
          site_domains[domain] = true
          site_to_domains[site_name] = site_domains
        else
          if not replace_base then
            utils.merge_into(result.by_domain[domain], entry)
          end

          result.by_domain[domain] = entry
        end
      end
    end

    local conflicted = {}
    for site, domains in pairs(site_to_domains) do
      domains = table_keys(domains)
      if #domains > 1 then
        domains = table.concat(domains, ', ')
        table.insert(conflicted, domains)
        print(
          'Multiple domains rollup to the same site: '
            .. site
            .. ' -> '
            .. domains
        )
      end
    end

    if #conflicted > 0 then
      -- This will generate a transient failure for every message
      -- until the issue is resolved
      error(
        'multiple conflicting rollup domains '
          .. table.concat(conflicted, ' ')
      )
    end

    -- print('Computed', kumo.json_encode_pretty(result))
    return result
  end

  local cached_load_data = kumo.memoize(load_shaping_data, {
    name = 'shaping_data',
    ttl = '5 minutes',
    capacity = 10,
  })

  local file_names = {
    -- './assets/policy-extras/shaping.toml',
    '/opt/kumomta/share/policy-extras/shaping.toml',
  }
  if extra_files then
    for _, filename in ipairs(extra_files) do
      table.insert(file_names, filename)
    end
  end

  local function get_egress_path_config(domain, egress_source, site_name)
    local data = cached_load_data(file_names)

    local by_site = data.by_site[site_name]
    local by_domain = data.by_domain[domain]

    local params = {}

    -- apply basic/default configuration
    utils.merge_into((data.by_domain['default'] or {}).params, params)

    -- then site config
    if by_site then
      utils.merge_into(by_site.params, params)
    end
    -- then domain config
    if by_domain then
      utils.merge_into(by_domain.params, params)
    end

    -- then source config for the site
    if by_site then
      utils.merge_into(by_site.sources[egress_source], params)
    end

    -- then source config for the domain
    if by_domain then
      utils.merge_into(by_domain.sources[egress_source], params)
    end

    -- print("going to make egress path", kumo.json_encode(params))

    return kumo.make_egress_path(params)
  end

  return get_egress_path_config
end

return mod
