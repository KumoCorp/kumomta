# Lua Resources

Lua is portugese for moon.  It is also the name of the scripting language we use in KumoMTA.  Because it is a name, Lua is always capitalised.

Understanding Lua is not absolutely required to deploy and use KumoMTA, but it will help you leverage the full power of this incredibly flexible system. Lua is fairly easy to learn, is easy to read, and is easy to implement.

You can find many resources at the [official Lua site](https://www.lua.org/home.html) including on-line documentation and physical reference books.

Here is a very (very) simplified priber to help you read KumoMTA script/configs.

## Simplified Lua Cheat Sheet
```
 -- A single line comment in Lua is 2 dashes (--)
 
--[[ A multi line comment in Lua 
     is framed in 2 dashes and 2 square brackets
     ]]--
```

Variables should always be declared as "local" unless you fully understand the affects of setting a global variable.  Global variables are dangerous.
```
local myvar
local myvar =  32
```

Global variables are implied by excluding the word "local".
``` 
myvar = 32 -- this is a GLOBAL variable
```

Lua supports the following relational operators:
```
==: equality
~=: inequality
<: less than
>: greater than
<=: less or equal
>=: greater or equal
```

You can concat strings with two dots surrounded by spaces.  
``` print("This" .. " is " .. "true.")```

Functions, conditionals and loops alwyas end with "end"
``` if x==2 then y=6 end```
```
function dostuff (things)
   print(things)
end
```


