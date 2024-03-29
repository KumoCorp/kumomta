local tsa = require 'tsa'
local kumo = require 'kumo'

local TEST_DIR = os.getenv 'KUMO_TSA_TEST_DIR'

kumo.on('tsa_init', function()
  tsa.start_http_listener {
    listen = '127.0.0.1:0',
  }
  tsa.configure_tsa_db_path(string.format('%s/tsa.db', TEST_DIR))
end)

local cached_load_shaping_data = kumo.memoize(kumo.shaping.load, {
  name = 'tsa_load_shaping_data',
  ttl = '5 minutes',
  capacity = 4,
})

kumo.on('tsa_load_shaping_data', function()
  local shaping = cached_load_shaping_data {
    'shaping.toml',
  }
  return shaping
end)
