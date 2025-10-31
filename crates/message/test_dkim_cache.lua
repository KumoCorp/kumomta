local kumo = require 'kumo'

local signer = kumo.dkim.rsa_sha256_signer {
  domain = 'example.com',
  selector = 's1',
  headers = { 'From', 'To' },
  key = 'example-private-dkim-key.pem',
}

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

-- Sign 100 times
for i = 1, 100 do
  local msg = new_msg 'Subject: hello\r\n\r\nHello'
  msg:dkim_sign(signer)
end

-- And observe that we only loaded the key once
local metrics = kumo.prometheus_metrics()
-- print(kumo.serde.json_encode_pretty(metrics))
assert(metrics.lruttl_lookup_count.value.cache_name.dkim_key_cache == 1)
assert(metrics.lruttl_lookup_count.value.cache_name.dkim_signer_cache == 1)

assert(metrics.lruttl_populated_count.value.cache_name.dkim_key_cache == 1)
assert(metrics.lruttl_populated_count.value.cache_name.dkim_signer_cache == 1)

assert(metrics.lruttl_evict_count.value.cache_name.dkim_key_cache == 0)
assert(metrics.lruttl_evict_count.value.cache_name.dkim_signer_cache == 0)

assert(metrics.dkim_signer_cache_lookup_count.value == 1)
