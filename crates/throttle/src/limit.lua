local now_ts = math.floor(tonumber(ARGV[1]))
local expires_ts = math.ceil(tonumber(ARGV[2]))
local limit = tonumber(ARGV[3])
local uuid = ARGV[4]

local DEBUG = false

-- prune expired values
if DEBUG then
  redis.log(
    redis.LOG_DEBUG,
    'limiter: going to call ZREMRANGEBYSCORE',
    KEYS[1],
    now_ts - 1
  )
end

local pruned = redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now_ts - 1)

if DEBUG then
  redis.log(redis.LOG_DEBUG, 'limiter: ZREMRANGEBYSCORE -> pruned=', pruned)
end

-- Count number of leases
if DEBUG then
  redis.log(redis.LOG_DEBUG, 'limiter: going to call ZCARD', KEYS[1])
end
local count = redis.call('ZCARD', KEYS[1])
if DEBUG then
  redis.log(redis.LOG_DEBUG, 'limiter: ZCARD', KEYS[1], ' -> count=', count)
end

if count + 1 > limit then
  -- too many: find the next expiration time
  if DEBUG then
    redis.log(
      redis.LOG_DEBUG,
      'limiter: going to call ZRANGEBYSCORE',
      KEYS[1]
    )
  end

  local smallest
  if redis.REDIS_VERSION_NUM >= 0x062000 then
    smallest = redis.call(
      'ZRANGE',
      KEYS[1],
      '-inf',
      '+inf',
      'BYSCORE',
      'LIMIT',
      0,
      1,
      'WITHSCORES'
    )
  else
    smallest = redis.call(
      'ZRANGEBYSCORE',
      KEYS[1],
      '-inf',
      '+inf',
      'WITHSCORES',
      'LIMIT',
      0,
      1
    )
  end

  if DEBUG then
    redis.log(
      redis.LOG_DEBUG,
      'limiter: ZRANGEBYSCORE',
      KEYS[1],
      ' -> ',
      cjson.encode(smallest)
    )
  end

  -- smallest holds the uuid and its expiration time;
  -- we want to just return the remaining time interval
  return math.ceil(smallest[2] - now_ts)
end

-- There's room for us to add/claim a lease

if DEBUG then
  redis.log(
    redis.LOG_DEBUG,
    'limiter: going to call ZADD',
    KEYS[1],
    expires_ts,
    uuid
  )
end

local result = redis.call('ZADD', KEYS[1], 'NX', expires_ts, uuid)

if DEBUG then
  redis.log(redis.LOG_DEBUG, 'limiter: ZADD', KEYS[1], ' -> ', result)
end

-- We got one
return redis.status_reply 'OK'
