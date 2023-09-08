local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local DKIM_PATH = '/opt/kumomta/etc/dkim'

--[[
Usage example:

local dkim_sign = require 'policy-extras.dkim_sign'
local dkim_signer = dkim_sign:setup({'/opt/kumomta/etc/dkim_data.toml'})

kumo.on('smtp_server_message_received', function(msg)
  dkim_signer(msg)
end)

kumo.on('http_message_generated', function(msg)
  dkim_signer(msg)
end)

]]

--[[
Example data file structure:

[base]
# If these are present, we'll use hashicorp vault instead
# of reading from disk
vault_mount = "secret"
vault_path_prefix = "dkim/"

# To do double or triple signing, add each additional
# signature name to this list and see the `signature."MyESPName"`
# block below
additional_signatures = ["MyESPName"]

# Default selector to assume if the domain/signature block
# doesn't specify one
selector = "dkim1024"

# The default set of headers to sign if otherwise unspecified
headers = ["From", "To", "Subject", "Date", "MIME-Version", "Content-Type", "Sender"]

# Domain blocks match based on the sender domain of the
# incoming message
[domain."example.com"]
selector = 'dkim1024'
headers = ["From", "To", "Subject", "Date", "MIME-Version", "Content-Type", "Sender"]
algo = "sha256" # or "ed25519". Omit to use the default of "sha256"

# optional overridden filename.
# Default is "/opt/kumomta/etc/dkim/DOMAIN/SELECTOR.key"
filename = "/full/path/to/key."

# TODO: reception-time policy for signing based on DNS.
policy = "TempFailIfNotInDNS" # Reject
#policy = "SignAlways"         # Sign and relay
#policy = "SignOnlyIfInDNS"    # Don't sign. Allow fallback to additional_signatures

# The signature block is independent of the sender domain.
# They are consulted based on the value of `base.additional_signatures`
# above.
# In addition to the same values that are found in the `domain` block,
# the following keys are supported
[signature."MyESPName"]
# Policy is interpreted differently for these
policy = "Always" # Always add this signature
#policy = "OnlyIfMissingDomainBlock" # Use this as a fallback

# specifies the signing domain for this signature block
domain = "myesp.com"
]]

local function load_dkim_data_from_file(file_name, target)
  local data = utils.load_json_or_toml_file(file_name)

  if data.base then
    utils.merge_into(data.base, target.base)
  end

  if data.domain then
    for domain, params in pairs(data.domain) do
      if not target.domain[domain] then
        target.domain[domain] = params
      else
        utils.merge_into(params, target.domain[domain])
      end
    end
  end

  if data.signature then
    for signame, params in pairs(data.signature) do
      if not target.signature[signame] then
        target.signature[signame] = params
      else
        utils.merge_into(params, target.signature[signame])
      end
    end
  end
end

local function load_dkim_data(dkim_data_files)
  local data = {
    domain = {},
    signature = {},
    base = {},
  }
  for _, file_name in ipairs(dkim_data_files) do
    load_dkim_data_from_file(file_name, data)
  end

  -- Sanity checks
  if data.base.additional_signatures then
    for _, signame in ipairs(data.base.additional_signatures) do
      if not data.signature[signame] then
        error(
          string.format(
            "dkim policy lists base.additional_signature '%s' but that signature is not defined in any signature block",
            signame
          )
        )
      end
    end
  end

  for domain_name, domain in pairs(data.domain) do
    if not data.base.selector and not domain.selector then
      error(
        string.format(
          "dkim domain '%s' is missing a selector and no default selector is defined in base",
          domain_name
        )
      )
    end
  end

  -- Compile the domain map for pattern matching
  data.domain = kumo.domain_map.new(data.domain)

  return data
end

local function make_signer(params, algo)
  algo = algo or 'sha256'

  if algo == 'sha256' then
    return kumo.dkim.rsa_sha256_signer(params)
  end

  if algo == 'ed25519' then
    return kumo.dkim.ed25519_signer(params)
  end

  error(
    string.format("invalid algo '%s' for domain '%s'", algo, params.domain)
  )
end

local function do_dkim_sign(msg, data)
  local sender_domain = msg:from_header().domain

  local signed_domain = false
  local domain_config = data.domain[sender_domain]
  local base = data.base

  if domain_config then
    -- TODO: check DNS to decide whether to try and sign based
    -- on the domain_config.policy value

    local params = {
      domain = sender_domain,
      selector = domain_config.selector or data.base.selector,
      headers = domain_config.headers or base.headers,
      header_canonicalization = domain_config.header_canonicalization
        or base.header_canonicalization,
      body_canonicalization = domain_config.body_canonicalization
        or base.body_canonicalization,
    }

    if base.vault_mount then
      params.key = {
        vault_mount = base.vault_mount,
        vault_path = domain_config.filename or string.format(
          '%s/%s/%s.key',
          base.vault_path_prefix or 'dkim',
          params.domain,
          params.selector
        ),
      }
    else
      params.key = domain_config.filename
        or string.format(
          '%s/%s/%s.key',
          DKIM_PATH,
          params.domain,
          params.selector
        )
    end

    local signer = make_signer(params, domain_config.algo)
    msg:dkim_sign(signer)
    signed_domain = true
  end

  if base.additional_signatures then
    for _, signame in ipairs(base.additional_signatures) do
      local sig_config = data.signature[signame]

      local need_sign = true
      if
        sig_config.policy == 'OnlyIfMissingDomainBlock' and signed_domain
      then
        -- Ideally we'd simply "continue" here, but lua doesn't have continue!
        need_sign = false
      end

      if need_sign then
        local params = {
          domain = sig_config.domain,
          selector = sig_config.selector or data.base.selector,
          headers = sig_config.headers or base.headers,
          header_canonicalization = sig_config.header_canonicalization
            or base.header_canonicalization,
          body_canonicalization = sig_config.body_canonicalization
            or base.body_canonicalization,
        }

        if base.vault_mount then
          params.key = {
            vault_mount = base.vault_mount,
            vault_path = sig_config.filename or string.format(
              '%s/%s/%s.key',
              base.vault_path_prefix or 'dkim',
              params.domain,
              params.selector
            ),
          }
        else
          params.key = sig_config.filename
            or string.format(
              '%s/%s/%s.key',
              DKIM_PATH,
              params.domain,
              params.selector
            )
        end

        local signer = make_signer(params, sig_config.algo)
        msg:dkim_sign(signer)
      end
    end
  end
end

function mod:setup(dkim_data_files)
  local cached_load_data = kumo.memoize(load_dkim_data, {
    name = 'dkim_signing_data',
    ttl = '5 minutes',
    capacity = 10,
  })

  local sign_message = function(msg)
    local data = cached_load_data(dkim_data_files)
    do_dkim_sign(msg, data)
  end

  return sign_message
end

return mod
