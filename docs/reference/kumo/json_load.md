# `kumo.json_load(FILENAME)`

Reads the content of the file name `FILENAME` and parses it as JSON,
returning a lua representation of the parsed JSON.

This json loading implementation will accept C style block comments, C++ style
single line comments and shell style single line comments.  Comments will be
treated as though they were spaces prior to being parsed by the underlying json
parser.

See also [kumo.json_parse](json_parse.md), [kumo.json_encode](json_encode.md)
and [kumo.json_encode_pretty](json_encode_pretty.md)
