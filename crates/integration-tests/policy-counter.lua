local kumo = require 'kumo'
local dir = os.getenv 'KUMOD_TEST_DIR'

kumo.on('init', function()
  kumo.configure_accounting_db_path ':memory:'
  kumo.aaa.configure_acct_log { log_dir = dir .. '/acct' }
  for _, name in ipairs { 'data', 'meta' } do
    kumo.define_spool {
      name = name,
      kind = 'RocksDB',
      path = dir .. '/' .. name,
    }
  end
  kumo.start_http_listener { listen = '127.0.0.1:0' }

  local checks = kumo.metrics.counter {
    name = 'rule_evaluations_total',
    labels = { 'rule' },
  }
  checks:inc { rule = 'quoted "rule"\npath\\' }
end)
