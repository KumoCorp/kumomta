local mod = {}
local kumo = require 'kumo'

-- Helper function that merges the values from `src` into `dest`
function mod.merge_into(src, dest)
  if src then
    for k, v in pairs(src) do
      dest[k] = v
    end
  end
end

-- Helper function to return the keys of a object style table as an array style
-- table
function mod.table_keys(tbl)
  local result = {}
  for k, v in pairs(tbl) do
    table.insert(result, k)
  end
  return result
end

function mod.load_json_or_toml_file(filename)
  if filename:match '.toml$' then
    return kumo.toml_load(filename)
  end
  return kumo.json_load(filename)
end

return mod
