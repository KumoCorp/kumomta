local mod = {}
local kumo = require 'kumo'

---@param o1 any|table First object to compare
---@param o2 any|table Second object to compare
---@param ignore_mt boolean True to ignore metatables (a recursive function to tests tables inside tables)
-- https://stackoverflow.com/a/32660766/149111
function mod.equals(o1, o2, ignore_mt)
  if o1 == o2 then
    return true
  end
  local o1Type = type(o1)
  local o2Type = type(o2)
  if o1Type ~= o2Type then
    return false
  end
  if o1Type ~= 'table' then
    return false
  end

  if not ignore_mt then
    local mt1 = getmetatable(o1)
    if mt1 and mt1.__eq then
      --compare using built in method
      return o1 == o2
    end
  end

  local keySet = {}

  for key1, value1 in pairs(o1) do
    local value2 = o2[key1]
    if value2 == nil or mod.equals(value1, value2, ignore_mt) == false then
      return false
    end
    keySet[key1] = true
  end

  for key2, _ in pairs(o2) do
    if not keySet[key2] then
      return false
    end
  end
  return true
end

function mod.assert_eq(a, b, message)
  if not mod.equals(a, b) then
    if message then
      message = string.format('%s. ', message)
    else
      message = ''
    end
    error(
      string.format(
        '%sexpected %s == %s',
        message,
        kumo.json_encode(a),
        kumo.json_encode(b)
      )
    )
  end
end

-- Helper function that merges the values from `src` into `dest`
function mod.merge_into(src, dest)
  if src then
    for k, v in pairs(src) do
      dest[k] = v
    end
  end
end

-- Helper function that merges the values from `src` into `dest`
function mod.recursive_merge_into(src, dest)
  if src then
    for k, v in pairs(src) do
      if type(v) == 'table' then
        if not dest[k] then
          dest[k] = v
        else
          mod.recursive_merge_into(v, dest[k])
        end
      else
        dest[k] = v
      end
    end
  end
end

-- Returns true if the table tbl contains value v
function mod.table_contains(tbl, v)
  if v == nil then
    return false
  end
  for _, value in pairs(tbl) do
    if value == v then
      return true
    end
  end
  return false
end

-- Helper function to return the keys of a object style table as an array style
-- table
function mod.table_keys(tbl)
  local result = {}
  for k, _v in pairs(tbl) do
    table.insert(result, k)
  end
  return result
end

-- Returns true if table is empty, false otherwise
function mod.table_is_empty(tbl)
  for _k, _v in pairs(tbl) do
    return false
  end
  return true
end

function mod.load_json_or_toml_file(filename)
  if filename:match '.toml$' then
    return kumo.toml_load(filename)
  end
  return kumo.json_load(filename)
end

-- Returns true if the string `s` starts with the string `text`
function mod.starts_with(s, text)
  return string.sub(s, 1, #text) == text
end

-- Returns true if the string `s` ends with the string `text`
function mod.ends_with(s, text)
  return string.sub(s, -#text) == text
end

-- Given an IP:port, split it into a tuple of IP and port
function mod.split_ip_port(ip_port)
  -- "127.0.0.1:25" -> "127.0.0.1", "25"
  -- "[::1]:25" -> "[::1]", "25"
  local ip, port = string.match(ip_port, '^(.*):(%d+)$')

  -- If we have `[ip]` for ip, remove the square brackets
  local remove_square = string.match(ip, '^%[(.*)%]$')
  if remove_square then
    return remove_square, port
  end

  return ip, port
end

return mod
