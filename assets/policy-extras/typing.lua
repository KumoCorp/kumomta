local mod = {}
local kumo = require 'kumo'

local function value_dump(v)
  if type(v) == 'table' then
    local status, encoded = pcall(kumo.serde.json_encode_pretty, v)
    if status then
      return encoded
    end
  end
  if type(v) == 'string' then
    return "'" .. v .. "'"
  end
  return v
end

-- Figure out the number of frames to skip in the error() call
-- when reporting an error.
-- We want to report the caller outside of this module,
-- but we do also want to correctly report the line number
-- of tests in this file.
-- We do this by looking at the stack trace and comparing
-- frames; the very first frame is by definition in this function,
-- so we can use that to figure out the current source file
-- and filter based on that.
local function error_skip()
  local trace = kumo.traceback(1)

  -- print(kumo.serde.json_encode_pretty(trace))
  local first_frame = trace[1]

  for level, frame in ipairs(trace) do
    if frame.what ~= 'C' then
      if frame.source ~= first_frame.source then
        -- It's not in this file
        return level, frame
      end
      -- This is fragile; the number here should be
      -- approximately the line number in this file where
      -- mod:test is defined. If the number is too low,
      -- we'll attribute errors to the typing helpers in
      -- this file, rather than the caller.
      if frame.curr_line > 580 then
        return level, frame
      end
    end
  end

  return 2, trace[2]
end

local TypeError = {}

function TypeError:format_frame()
  return string.format(
    '%s:%d %s',
    self.frame.short_src,
    self.frame.curr_line,
    self.message
  )
end

function TypeError:__tostring()
  if self.context then
    local frames = { self:format_frame() }
    local context = self.context
    while context do
      table.insert(frames, context:format_frame())
      context = context.context
    end
    return table.concat(frames, '\n')
  else
    return self:format_frame()
  end
end

function TypeError:raise()
  error(tostring(self), error_skip())
end

TypeError.__index = TypeError

function TypeError:new(message, context)
  if not message then
    error('TypeError:new called with no message', 2)
  end

  --[[
  print("TypeError:new called with message", kumo.json_encode_pretty(message))
  print("TypeError:new called with context", kumo.json_encode_pretty(context))
  ]]

  if context then
    if type(context) ~= 'table' then
      context = TypeError:new(context)
    end
  end

  local skip, frame = error_skip()

  local err = {
    message = message,
    context = context,
    frame = frame,
  }
  -- print("TypeError:new -> ", kumo.json_encode_pretty(err))
  setmetatable(err, self)
  return err
end

local function casting_validate_value(ty, other)
  local other_mt = getmetatable(other)
  if other_mt == ty then
    return true, other
  end

  if not other_mt then
    if type(other) == 'table' then
      -- Let's try to construct one implicitly
      local status, fixup = pcall(ty.construct, ty, other)
      if not status then
        return false,
          TypeError:new(
            string.format(
              "Expected value of type '%s' and encountered an error during coercion",
              ty.name
            ),
            fixup
          )
      end
      return true, fixup
    end

    return false,
      TypeError:new(
        string.format(
          "Expected value of type '%s' but got type '%s' %s",
          ty.name,
          type(other),
          value_dump(other)
        )
      )
  end

  if other_mt.name then
    return false,
      TypeError:new(
        string.format(
          "Expected value of type '%s' but got type '%s' with value %s",
          ty.name,
          other_mt.name,
          value_dump(other)
        )
      )
  end

  return false,
    TypeError:new(
      string.format(
        "Expected value of type '%s' but got type '%s' %s",
        ty.name,
        type(other),
        value_dump(other)
      )
    )
end

-- Returns a record constructor
--
-- fields is a map<string, type>, where the key is the field
-- name and the value is the type of that field.
--
-- _dynamic is a special entry: it enables performing a runtime
-- check to see if a given key/value pair is a valid value for
-- assignment. This is used to outsource validation to native
-- rust code in the embedding application.  The value in that
-- case must be a function that accepts the key and value
-- and returns a boolean to indicate whether that combination
-- is acceptable.
--
-- { _dynamic = function(key, value) return true end }
function mod.record(name, fields)
  local ty = {
    name = name,
    fields = fields,
  }

  function ty.__index(t, k)
    local v = rawget(t, k)
    if v then
      return v
    end
    local field_def = ty.fields[k]
    if not field_def then
      TypeError
        :new(
          string.format("%s: attempt to read unknown field '%s'", ty.name, k)
        )
        :raise()
    end
  end

  function ty.__newindex(t, k, v)
    local field_def = ty.fields[k]
    if not field_def then
      local dyn_validate = ty.fields._dynamic
      if dyn_validate and dyn_validate(k, v) then
        rawset(t, k, v)
        return
      end

      TypeError:new(string.format("%s: unknown field '%s'", ty.name, k))
        :raise()
    end
    local status, result = field_def:validate_value(v)
    if status then
      rawset(t, k, result)
    else
      TypeError
        :new(
          string.format("%s: invalid value for field '%s'", ty.name, k),
          result
        )
        :raise()
    end
  end

  function ty:construct(params)
    local obj = {}
    setmetatable(obj, ty)

    for k, v in pairs(params) do
      obj[k] = v
    end

    for k, def in pairs(ty.fields) do
      if not obj[k] then
        if type(def) == 'table' then
          if def.default_value then
            obj[k] = def.default_value
          elseif not def.is_optional then
            TypeError
              :new(
                string.format(
                  "%s: missing value for field '%s' of type '%s'",
                  ty.name,
                  k,
                  def.name
                )
              )
              :raise()
          end
        end
      end
    end

    return obj
  end

  function ty:validate_value(other)
    return casting_validate_value(ty, other)
  end

  local ctor_mt = {}
  setmetatable(ty, ctor_mt)

  function ctor_mt:__call(params)
    return self:construct(params)
  end

  return ty
end

function mod.list(value_type)
  local ty = {
    name = string.format('list<%s>', value_type.name),
    value_type = value_type,
  }

  function ty:construct(params)
    local obj = {}
    setmetatable(obj, ty)

    for i, v in ipairs(params) do
      obj[i] = v
    end

    return obj
  end

  function ty.__newindex(t, idx, v)
    if type(idx) ~= 'number' then
      TypeError
        :new(
          string.format(
            "%s: invalid index '%s' list",
            ty.name,
            value_dump(idx)
          )
        )
        :raise()
    end

    local val_ok, val_fixup = ty.value_type:validate_value(v)
    if not val_ok then
      TypeError
        :new(
          string.format('%s: invalid value for idx %d', ty.name, idx),
          val_fixup
        )
        :raise()
    end

    rawset(t, idx, val_fixup)
  end

  function ty:validate_value(other)
    return casting_validate_value(ty, other)
  end

  local ctor_mt = {}
  setmetatable(ty, ctor_mt)

  function ctor_mt:__call(params)
    return self:construct(params)
  end

  return ty
end

function mod.map(key_type, value_type)
  local ty = {
    name = string.format('map<%s,%s>', key_type.name, value_type.name),
    key_type = key_type,
    value_type = value_type,
  }

  function ty:construct(params)
    local obj = {}
    setmetatable(obj, ty)

    for k, v in pairs(params) do
      obj[k] = v
    end

    return obj
  end

  function ty.__newindex(t, k, v)
    local key_ok, key_fixup = ty.key_type:validate_value(k)
    if not key_ok then
      TypeError
        :new(
          string.format('%s: invalid key %s', ty.name, value_dump(k)),
          key_fixup
        )
        :raise()
    end

    local val_ok, val_fixup = ty.value_type:validate_value(v)
    if not val_ok then
      TypeError
        :new(
          string.format(
            '%s: invalid value for key %s',
            ty.name,
            value_dump(k)
          ),
          val_fixup
        )
        :raise()
    end

    rawset(t, key_fixup, val_fixup)
  end

  function ty:validate_value(other)
    return casting_validate_value(ty, other)
  end

  local ctor_mt = {}
  setmetatable(ty, ctor_mt)

  function ctor_mt:__call(params)
    return self:construct(params)
  end

  return ty
end

local function make_simple_ctor(ty)
  local ctor_mt = {}
  setmetatable(ty, ctor_mt)
  function ctor_mt:__call(value)
    local status, fixup = ty:validate_value(value)
    if not status then
      error(err, error_skip())
    end
    return fixup
  end
  return ty
end

local function make_scalar(type_name)
  local ty = {
    name = type_name,
  }

  function ty:validate_value(v)
    if type(v) ~= type_name then
      return false,
        TypeError:new(
          string.format(
            "Expected '%s', got '%s' %s",
            type_name,
            type(v),
            value_dump(v)
          )
        )
    end
    return true, v
  end

  return make_simple_ctor(ty)
end

mod.number = make_scalar 'number'
mod.string = make_scalar 'string'
mod.boolean = make_scalar 'boolean'

function mod.enum(name, ...)
  local variants = { ... }
  local is_valid = {}
  for _, v in ipairs(variants) do
    is_valid[v] = true
  end

  local ty = {
    name = name,
    variants = { ... },
    is_valid = is_valid,
  }

  function ty:validate_value(v)
    if not self.is_valid[v] then
      local list = {}
      for _, valid in ipairs(self.variants) do
        table.insert(list, value_dump(valid))
      end
      return false,
        TypeError:new(
          string.format(
            "Unexpected '%s' value %s, expected one of %s",
            ty.name,
            value_dump(v),
            table.concat(list, ', ')
          )
        )
    end
    return true, v
  end

  return make_simple_ctor(ty)
end

function mod.default(target_type, default_value)
  local ty = {
    name = target_type.name,
    default_value = default_value,
    target = target_type,
  }

  function ty:validate_value(v)
    if v == nil then
      return self.default_value
    end
    local status, error = self.target:validate_value(v)
    return status, error
  end

  return make_simple_ctor(ty)
end

function mod.option(target_type)
  local ty = {
    name = string.format('option<%s>', target_type.name),
    is_optional = true,
    target = target_type,
  }

  function ty:validate_value(v)
    if v == nil then
      return true
    end
    local status, error = self.target:validate_value(v)
    return status, error
  end

  return make_simple_ctor(ty)
end

function mod:test()
  local utils = require 'policy-extras.policy_utils'

  local Layer = mod.enum('Layer', 'Above', 'Below')

  local Point = mod.record('Point', {
    x = mod.number,
    y = mod.number,
  })

  local Example = mod.record('Example', {
    point = Point,
    layer = mod.option(Layer),
  })

  -- Check that we can construct with a nested record type
  local pt = Point { x = 123, y = 2.5 }
  local a = Example { point = pt, layer = 'Above' }
  utils.assert_eq(a, { point = { y = 2.5, x = 123 }, layer = 'Above' })

  -- Check that we can construct with the optional field
  local a = Example { point = pt }
  utils.assert_eq(a, { point = { y = 2.5, x = 123 } })

  -- Check that we can construct with an implicit, inline
  -- record type (the point)
  local b = Example { point = { x = 123, y = 4 }, layer = 'Above' }
  utils.assert_eq(b, { point = { y = 4, x = 123 }, layer = 'Above' })

  -- Check error for missing field
  local status, err = pcall(Point, { x = 123 })
  assert(not status)
  utils.assert_matches(
    err,
    "Point: missing value for field 'y' of type 'number'"
  )

  -- Check invalid type assignment
  local status, err = pcall(Point, { x = 123, y = true })
  assert(not status)
  utils.assert_matches(
    err,
    "Point: invalid value for field 'y'\n.*Expected 'number', got 'boolean' true"
  )

  local status, err =
    pcall(Example, { point = { x = 123, y = 4 }, layer = 'Wrong' })
  assert(not status)
  utils.assert_matches(
    err,
    "Example: invalid value for field 'layer'\n.*Unexpected 'Layer' value 'Wrong', expected one of 'Above', 'Below'"
  )

  local Foo = mod.record('Foo', {
    ex = Example,
  })

  local status, err = pcall(Foo, { ex = Point { x = 123, y = 4 } })
  assert(not status)
  utils.assert_matches(
    err,
    "Foo: invalid value for field 'ex'\n.*Expected value of type 'Example' but got type 'Point' with value"
  )

  local StringMap = mod.map(mod.string, Example)
  local m = StringMap { a = a, b = b }
  utils.assert_eq(m, { a = a, b = b })

  -- Check map assignment respects key type
  local status, err = pcall(StringMap, { [123] = a })
  assert(not status)
  utils.assert_matches(
    err,
    "map<string,Example>: invalid key 123\n.*Expected 'string', got 'number' 123"
  )

  local status, err = pcall(function()
    local map = StringMap {}
    map[123] = 'boo'
  end)
  assert(not status)
  utils.assert_matches(
    err,
    "map<string,Example>: invalid key 123\n.*Expected 'string', got 'number' 123"
  )

  -- Check map assignment respects value type
  local status, err = pcall(StringMap, { hello = 'wrong' })
  assert(not status)
  utils.assert_matches(
    err,
    "map<string,Example>: invalid value for key 'hello'\n.*Expected value of type 'Example' but got type 'string' 'wrong'"
  )

  -- Verify that defaulting works
  local WithDefaultLayer = mod.record('WithDefaultLayer', {
    layer = mod.default(Layer, 'Above'),
  })
  local have_default_layer = WithDefaultLayer {}
  assert(have_default_layer.layer == 'Above')

  local StringList = mod.list(mod.string)
  utils.assert_eq(StringList {}, {})
  utils.assert_eq(StringList { 'hello', 'there' }, { 'hello', 'there' })

  local status, err = pcall(StringList, { 1, 2, 3 })
  assert(not status)
  utils.assert_matches(
    err,
    "list<string>: invalid value for idx 1\n.*Expected 'string', got 'number' 1"
  )
end

return mod
