local mod = {}
local kumo = require 'kumo'

local function value_dump(v)
  if type(v) == 'table' then
    local status, encoded = pcall(kumo.serde.json_encode_pretty, v)
    if status then
      return encoded
    end
  end
  return v
end

-- Returns a record constructor
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
      error(
        string.format("%s: attempt to read unknown field '%s'", ty.name, k),
        2
      )
    end
  end

  function ty.__newindex(t, k, v)
    local field_def = ty.fields[k]
    if not field_def then
      error(string.format("%s: unknown field '%s'", ty.name, k), 2)
    end
    local status, result = field_def:validate_value(v)
    if status then
      rawset(t, k, result)
    else
      error(
        string.format(
          "%s: invalid value '%s' for field '%s': %s",
          ty.name,
          value_dump(v),
          k,
          result
        ),
        2
      )
    end
  end

  function ty:construct(params)
    local obj = {}
    setmetatable(obj, ty)

    for k, v in pairs(params) do
      obj[k] = v
    end

    for k, def in pairs(ty.fields) do
      if not obj[k] and not def.is_optional then
        error(
          string.format(
            "%s: missing value for field '%s' of type '%s'",
            ty.name,
            k,
            def.name
          ),
          2
        )
      end
    end

    return obj
  end

  function ty:validate_value(other)
    local other_mt = getmetatable(other)
    if other_mt == self then
      return true, other
    end

    if not other_mt then
      if type(other) == 'table' then
        -- Let's try to construct one implicitly
        local status, fixup = pcall(self.construct, self, other)
        if not status then
          return false,
            string.format(
              "Expected value of type '%s' and encountered an error during construction:\n%s",
              self.name,
              fixup
            )
        end
        return true, fixup
      end

      return false,
        string.format(
          "Expected value of type '%s' but got '%s'",
          self.name,
          value_dump(other)
        )
    end

    if other_mt.name then
      return false,
        string.format(
          "Expected value of type '%s' but got type '%s' with value '%s'",
          self.name,
          other_mt.name,
          value_dump(other)
        )
    end

    return false,
      string.format(
        "Expected value of type '%s' but got '%s'",
        self.name,
        value_dump(other)
      )
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
      error(
        string.format("%s: invalid index '%s' list", ty.name, value_dump(idx)),
        2
      )
    end

    local val_ok, val_fixup = ty.value_type:validate_value(v)
    if not val_ok then
      error(
        string.format(
          "%s: invalid value '%s' for idx %d: %s",
          ty.name,
          value_dump(v),
          idx,
          val_fixup
        ),
        2
      )
    end

    rawset(t, idx, val_fixup)
  end

  function ty:validate_value(other)
    local other_mt = getmetatable(other)
    if other_mt == self then
      return true, other
    end

    if not other_mt then
      if type(other) == 'table' then
        -- Let's try to construct one implicitly
        local status, fixup = pcall(self.construct, self, other)
        if not status then
          return false,
            string.format(
              "Expected value of type '%s' and encountered an error during construction: %s",
              self.name,
              fixup
            )
        end
        return true, fixup
      end

      return false,
        string.format(
          "Expected value of type '%s' but got '%s'",
          self.name,
          value_dump(other)
        )
    end

    if other_mt.name then
      return false,
        string.format(
          "Expected value of type '%s' but got type '%s' with value '%s'",
          self.name,
          other_mt.name,
          value_dump(other)
        )
    end

    return false,
      string.format(
        "Expected value of type '%s' but got '%s'",
        self.name,
        value_dump(other)
      )
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
      error(
        string.format(
          "%s: invalid key '%s': %s",
          ty.name,
          value_dump(k),
          key_fixup
        ),
        2
      )
    end

    local val_ok, val_fixup = ty.value_type:validate_value(v)
    if not val_ok then
      error(
        string.format(
          "%s: invalid value '%s' for key '%s': %s",
          ty.name,
          value_dump(v),
          value_dump(k),
          val_fixup
        ),
        2
      )
    end

    rawset(t, key_fixup, val_fixup)
  end

  function ty:validate_value(other)
    local other_mt = getmetatable(other)
    if other_mt == self then
      return true, other
    end

    if not other_mt then
      if type(other) == 'table' then
        -- Let's try to construct one implicitly
        local status, fixup = pcall(self.construct, self, other)
        if not status then
          return false,
            string.format(
              "Expected value of type '%s' and encountered an error during construction: %s",
              self.name,
              fixup
            )
        end
        return true, fixup
      end

      return false,
        string.format(
          "Expected value of type '%s' but got '%s'",
          self.name,
          value_dump(other)
        )
    end

    if other_mt.name then
      return false,
        string.format(
          "Expected value of type '%s' but got type '%s' with value '%s'",
          self.name,
          other_mt.name,
          value_dump(other)
        )
    end

    return false,
      string.format(
        "Expected value of type '%s' but got '%s'",
        self.name,
        value_dump(other)
      )
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
      error(err, 2)
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
        string.format("expected '%s', got '%s'", type_name, type(v))
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
        table.insert(list, string.format("'%s'", valid))
      end
      return false,
        string.format(
          "unexpected '%s' value '%s', expected one of %s",
          ty.name,
          value_dump(v),
          table.concat(list, ', ')
        )
    end
    return true, v
  end

  return make_simple_ctor(ty)
end

-- TODO: mod.default - optional, with default value

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
  local Layer = mod.enum('Layer', 'Above', 'Below')

  local Point = mod.record('Point', {
    x = mod.number,
    y = mod.number,
  })

  local Example = mod.record('Example', {
    point = Point,
    layer = mod.option(Layer),
  })

  local pt = Point { x = 123, y = 2.5 }

  local a = Example { point = pt, layer = 'Above' }
  print('a is ', kumo.serde.json_encode_pretty(a))

  local b = Example { point = Point { x = 123, y = 4 }, layer = 'Above' }
  print('b is ', kumo.serde.json_encode_pretty(b))
  -- print(a.woot)

  local StringMap = mod.map(mod.string, Example)
  local m = StringMap { a = a, b = b }
end

if os.getenv 'KUMOMTA_RUN_UNIT_TESTS' then
  kumo.configure_accounting_db_path(os.tmpname())
  mod:test()
  os.exit(0)
end

return mod
