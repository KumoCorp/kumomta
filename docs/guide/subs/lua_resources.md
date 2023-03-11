# Lua Resources

Lua is portugese for moon.  It is also the name of the scripting language we use in KumoMTA.  Because it is a name, Lua is always capitalized.

Understanding Lua is not required to deploy and use KumoMTA, but it will help you leverage the full power of this incredibly flexible system. Lua is easy to learn, easy to read, and easy to implement.

You can find many resources at the [official Lua [site](https://www.lua.org/home.html) including online documentation and physical reference books.

Here is a very (very) simplified primer to help you read the KumoMTA script/configs.

## Simplified Lua Cheat Sheet

```lua
-- A single line comment in Lua is 2 dashes (--)

--[[ A multi line comment in Lua
     is framed in 2 dashes and 2 square brackets
   ]]
--
```

```admonish danger
Variables should always be declared as "local" unless you fully understand the affects of setting a global variable.  Global variables are dangerous.
```

```lua
local myvar
local myvar = 32
```

Global variables are implied by excluding the word "local".

```lua
myvar = 32 -- this is a GLOBAL variable
```

Lua supports the following relational operators:

| == | equality                 |
| -- | ------------------------ |
| ~= | inequality               |
| <  | less than                |
| >  | greater than             |
| <= | less than or equal to    |
| >= | greater than or equal to |

You can concatenate strings with two dots surrounded by spaces.

```lua
print('This' .. ' is ' .. 'true.')
```

Functions, conditionals, and loops always end with "end"

```lua
if x == 2 then
  y = 6
end

function dostuff(things)
  print(things)
end
```
