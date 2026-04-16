# kumo.file_type.from_media_type

```lua
local ft_list = kumo.file_type.from_media_type(EXT)
```

{{since('2025.12.02-67ee9e96')}}

Returns the list of file types that have the specified MIME media type.

This function will return an array style table holding one entry for
each file type that has the specified media type.

Each element of the array is a [FileTypeResult](index.md#filetyperesult).

```lua
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local png = kumo.file_type.from_media_type 'image/png'
-- Note that we're just looking at the first element of
-- the returned array here; there are a number of entries
-- returned for this media type
utils.assert_eq(png[1], {
  name = 'Portable Network Graphics',
  media_types = { 'image/png' },
  extensions = { 'png' },
})
```

