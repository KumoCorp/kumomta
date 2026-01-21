-- long running tests are skipped unless env variable AUDIT_SERIES_DEBUG is defined.

local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local P = {}

function P.define_test()
  local ok, res = pcall(kumo.audit_series.define, 'test_count', {
    window_count = 3,
    ttl = 300,
  })

  utils.assert_eq(true, ok)
  utils.assert_eq(nil, res)

  ok, res = pcall(kumo.audit_series.define, 'test_count', {
    window_count = 3,
    ttl = 300,
  })
  assert(
    string.find(tostring(res), 'is already defined'),
    'expected error "is already defined"'
  )
end

function P.clear_test()
  local ok, res = pcall(kumo.audit_series.define, 'test_clear', {
    window_count = 3,
    ttl = 300,
  })

  utils.assert_eq(true, ok)
  utils.assert_eq(nil, res)

  ok, res = pcall(kumo.audit_series.add, 'test_clear', 'entity1', 1)
  utils.assert_eq(true, ok)
  utils.assert_eq(1, res)
  ok, _ = pcall(kumo.audit_series.reset, 'test_clear', 'entity1')
  utils.assert_eq(true, ok)
  ok, res = pcall(kumo.audit_series.get, 'test_clear', 'entity1')
  utils.assert_eq(true, ok)
  utils.assert_eq(0, res)
end

function P.add_and_sub_test()
  local ok, res = pcall(kumo.audit_series.define, 'test_count_add_sub', {
    window_count = 2,
    ttl = '5 second',
  })

  utils.assert_eq(true, ok)
  utils.assert_eq(nil, res)

  ok, res = pcall(kumo.audit_series.add, 'test_count_add_sub', 'entity1', 1)
  utils.assert_eq(true, ok)
  utils.assert_eq(1, res, 'expected 1 after adding 1')

  -- subtract 1, we should get 0
  ok, res = pcall(kumo.audit_series.add, 'test_count_add_sub', 'entity1', -1)
  utils.assert_eq(true, ok)
  utils.assert_eq(0, res, 'expected 0 after subtracting 1')
end

function P.add_and_get_test()
  -- Run only in debug test
  if not os.getenv 'AUDIT_SERIES_DEBUG' then
    return
  end
  local ok, res = pcall(kumo.audit_series.define, 'add_and_get_test', {
    window_count = 2,
    ttl = '5 second',
  })

  utils.assert_eq(true, ok)
  utils.assert_eq(nil, res)

  -- test the window rolling over, make sure we don't continue to accumulate
  for i = 1, 5 do
    ok, res = pcall(kumo.audit_series.add, 'add_and_get_test', 'entity1', 1)
    utils.assert_eq(true, ok)
    utils.assert_eq(1, res, 'expected 1 after adding 1')

    -- call get for accumulated total
    ok, res = pcall(kumo.audit_series.get, 'add_and_get_test', 'entity1')
    utils.assert_eq(true, ok)
    local expected = i
    -- we are sleeping each loop, so each window would have 1
    if i > 2 then
      expected = 2
    end
    utils.assert_eq(expected, res, 'get is not returning expected count')
    kumo.time.sleep(5)
  end
end

function P.concurrent_test()
  -- Run only in debug test
  if not os.getenv 'AUDIT_SERIES_DEBUG' then
    return
  end
  kumo.time.sleep(2) -- wait for other tasks to complete
  local ok, res =
    pcall(kumo.audit_series.get, 'test_concurrent', 'entity_concurrent')
  utils.assert_eq(1000 * 50, res, 'concurrent add test failed')
end

kumo.on('my.task', function(args)
  -- Run only in debug test
  if not os.getenv 'AUDIT_SERIES_DEBUG' then
    return
  end

  for i = 1, 1000 do
    kumo.audit_series.add('test_concurrent', 'entity_concurrent', 1)
  end
end)

kumo.on('main', function()
  kumo.audit_series.define('test_concurrent', {
    window_count = 2,
    ttl = '60 second',
  })
  -- start up concurrent tasks all updating the metric
  -- this would be used by concurrent_test function
  for _ = 1, 50 do
    kumo.spawn_task {
      event_name = 'my.task',
      args = {},
    }
  end

  for name, func in pairs(P) do
    func()
  end
end)
