# `message:get_all_named_header_values(NAME)`

Gets the all the headers whose name matches `NAME`, decode them to UTF-8 and
return them in a lua array style table.

Returns an empty table if no matching headers were found.

