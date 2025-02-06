local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local typing = require 'policy-extras.typing'
local Bool, List, Map, Number, Option, Record, String =
  typing.boolean,
  typing.list,
  typing.map,
  typing.number,
  typing.option,
  typing.record,
  typing.string

local DKIM_PATH = '/opt/kumomta/etc/dkim'

local DomainSigningPolicy = typing.enum('DomainSigningPolicy', 'Always')
local SignatureSigningPolicy =
  typing.enum('SignatureSigningPolicy', 'Always', 'OnlyIfMissingDomainBlock')
local SigningAlgo = typing.enum('SigningAlgo', 'sha256', 'ed25519')

local DkimSignConfig = Record('DkimSignConfig', {
  base = Option(Record('DkimSignConfig.Base', {
    vault_mount = Option(String),
    vault_path_prefix = Option(String),
    additional_signatures = Option(List(String)),
    policy = Option(DomainSigningPolicy),
    selector = Option(String),
    expiration = Option(Number),
    headers = Option(List(String)),
    header_canonicalization = Option(String),
    body_canonicalization = Option(String),
    over_sign = Option(Bool),
  })),

  domain = Option(typing.map(
    String,
    Record('DkimSignConfig.Domain', {
      selector = Option(String),
      headers = Option(List(String)),
      policy = Option(DomainSigningPolicy),
      algo = Option(SigningAlgo),
      expiration = Option(Number),
      filename = Option(String),
      header_canonicalization = Option(String),
      body_canonicalization = Option(String),
      over_sign = Option(Bool),
    })
  )),

  signature = Option(typing.map(
    String,
    Record('DkimSignConfig.Signature', {
      domain = String,
      selector = Option(String),
      headers = Option(List(String)),
      policy = Option(SignatureSigningPolicy),
      algo = Option(SigningAlgo),
      expiration = Option(Number),
      filename = Option(String),
      header_canonicalization = Option(String),
      body_canonicalization = Option(String),
      over_sign = Option(Bool),
    })
  )),
})

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
  local raw_data = utils.load_json_or_toml_file(file_name)
  local is_ok, data = pcall(DkimSignConfig, raw_data)
  if not is_ok then
    error(string.format("reading data from file '%s': %s'", file_name, data))
  end

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

local function load_dkim_data(dkim_data_files, no_compile)
  local data = DkimSignConfig {
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
  if not no_compile then
    data.domain = kumo.domain_map.new(data.domain)
  end

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
  local from_header = msg:from_header()
  if not from_header then
    kumo.reject(
      552,
      '5.6.0 DKIM signing requires a From header, but it is missing from this message'
    )
  end
  local sender_domain = from_header.domain

  local signed_domain = false
  local domain_config = data.domain[sender_domain]
  local base = data.base

  if domain_config then
    -- TODO: check DNS to decide whether to try and sign based
    -- on the domain_config.policy value

    local policy = domain_config.policy or data.base.policy or 'Always'

    if policy ~= 'Always' then
      error(
        string.format(
          "dkim_sign: invalid policy '%s' for domain '%s'",
          policy,
          sender_domain
        )
      )
    end

    local params = {
      domain = sender_domain,
      selector = domain_config.selector or data.base.selector,
      expiration = domain_config.expiration or data.base.expiration,
      headers = domain_config.headers or base.headers,
      header_canonicalization = domain_config.header_canonicalization
        or base.header_canonicalization,
      body_canonicalization = domain_config.body_canonicalization
        or base.body_canonicalization,
      over_sign = domain_config.over_sign or base.over_sign,
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

      local policy = sig_config.policy or 'Always'

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
          expiration = sig_config.expiration or data.base.expiration,
          headers = sig_config.headers or base.headers,
          header_canonicalization = sig_config.header_canonicalization
            or base.header_canonicalization,
          body_canonicalization = sig_config.body_canonicalization
            or base.body_canonicalization,
          over_sign = sig_config.over_sign or base.over_sign,
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
  if mod.CONFIGURED then
    error 'dkim_sign module has already been configured'
  end

  local cached_load_data = kumo.memoize(load_dkim_data, {
    name = 'dkim_signing_data',
    ttl = '5 minutes',
    capacity = 10,
    invalidate_with_epoch = true,
  })

  local sign_message = function(msg)
    local data = cached_load_data(dkim_data_files)
    do_dkim_sign(msg, data)
  end

  mod.CONFIGURED = {
    data_files = dkim_data_files,
  }

  return sign_message
end

kumo.on('validate_config', function()
  if not mod.CONFIGURED then
    return
  end

  local failed = false

  function show_context()
    if failed then
      return
    end
    failed = true
    kumo.validation_failed()
    print 'Issues found in the combined set of dkim_sign files:'
    for _, file_name in ipairs(mod.CONFIGURED.data_files) do
      print(string.format(' - %s', file_name))
    end
  end

  local status, result =
    pcall(load_dkim_data, mod.CONFIGURED.data_files, true)
  if not status then
    show_context()
    print('Error loading data: ' .. result)
    return
  end

  local data = result
  local data_compiled = load_dkim_data(mod.CONFIGURED.data_files)
  -- print(kumo.json_encode_pretty(data))

  for domain, params in pairs(data.domain) do
    local msg = kumo.make_message(
      string.format('someone@%s', domain),
      'postmaster@example.com',
      string.format('From: someone@%s\r\nSubject: hello\r\n\r\nWoot', domain)
    )

    local status, err = pcall(do_dkim_sign, msg, data_compiled)
    if not status then
      show_context()
      print(string.format("domain '%s': %s", domain, err))
    end
  end

  local referenced = {}
  for _, signame in ipairs(data.base.additional_signatures or {}) do
    if not data.signature[signame] then
      show_context()
      print(
        string.format(
          "base.additional_signatures contains '%s' which does not have a corresponding [signature.'%s'] block",
          signame,
          signame
        )
      )
    end
    referenced[signame] = true
  end
  for signame, sigdata in pairs(data.signature or {}) do
    if not referenced[signame] then
      show_context()
      print(
        string.format(
          "[signature.'%s'] is not referenced by base.additional_signatures and will not be used",
          signame
        )
      )
    end
  end

  local msg = kumo.make_message(
    'someone@a.domain.that.really.should.not.exist.in.the.real.world.kumomta.com',
    'postmaster@example.com',
    'From: someone@a.domain.that.really.should.not.exist.in.the.real.world.kumomta.com\r\nSubject: hello\r\n\r\nWoot'
  )
  local status, err = pcall(do_dkim_sign, msg, data_compiled)
  if not status then
    show_context()
    print(string.format('Checking additional_signatures: %s', err))
  end
end)

return mod
