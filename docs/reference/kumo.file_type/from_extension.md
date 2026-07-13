# kumo.file_type.from_extension

```lua
local ft_list = kumo.file_type.from_extension(EXT)
```

{{since('2025.12.02-67ee9e96')}}

Returns the list of file types that have the specified filename extension.

This function will return an array style table holding one entry for
each file type that has the specified filename extension.

Each element of the array is a [FileTypeResult](index.md#filetyperesult).

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local markdown = kumo.file_type.from_extension 'markdown'
utils.assert_eq(markdown, {
  {
    name = 'Q1193600',
    media_types = {
      'text/markdown',
    },
    extensions = {
      'markdown',
      'md',
      'mdown',
      'mdtext',
      'mdtxt',
      'mkd',
    },
  },
})
```

