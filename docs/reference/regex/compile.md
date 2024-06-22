# `kumo.regex.compile(PATTERN)`

{{since('dev')}}

Compiles the regular expression `PATTERN`.
The supported syntax is [described in the Rust regex crate
documentation](https://docs.rs/regex/latest/regex/index.html#syntax), augmented
by [fancy regex
extensions](https://docs.rs/fancy-regex/latest/fancy_regex/#syntax), and is
similar to Perl-Compatible Regex, with a few differences.

!!! note
    In lua string literals backslash **must always** be escaped by a backslash,
    so you will need to write `\\` in cases where in other languages you
    might have been able to get away with just a single backslash.

The return from this function is a `Regex` object that has the following
methods:

## regex:captures(HAYSTACK)

Searches for the first match of this regex in the haystack given, and if found,
returns a table keyed by the captures defined in the regex.  Index 0 corresponds
to the full match. Each subsequent capture group is indexed by the order of its
opening `(`.

```lua
local re = kumo.regex.compile "'([^']+)'\\s+\\((\\d{4})\\)"
local cap = re:captures "Not my favorite movie: 'Citizen Kane' (1941)."
assert(cap[0] == "'Citizen Kane' (1941)")
assert(cap[1] == 'Citizen Kane')
assert(cap[2] == '1941')
```

Named capture groups are also supported; in addition to the numeric indices
described above, if you have used a named capture group, you can also index
the result by its name:

```lua
local re = kumo.regex.compile "'(?<title>[^']+)'\\s+\\((?<year>\\d{4})\\)"
local cap = re:captures "Not my favorite movie: 'Citizen Kane' (1941)."
assert(cap[0] == "'Citizen Kane' (1941)")
assert(cap.title == 'Citizen Kane')
assert(cap.year == '1941')
```

## regex:is_match(HAYSTACK)

Returns true if and only if there is a match for the regex anywhere in the
haystack given.

It is recommended to use this method if all you need to do is test whether a
match exists, since the underlying matching engine may be able to do less work.

```lua
local re = kumo.regex.compile 'oo'
assert(re:is_match 'foo')
```

## regex:find(HAYSTACK)

Searches for the first match of this regex in the haystack given, and if found,
returns it as a string.

```lua
local re = kumo.regex.compile 'o+'
assert(re:find 'food' == 'oo')
assert(re:find 'fooood' == 'oooo')
```

## regex:find_all(HAYSTACK)

Searchs for successive non-overlapping matches in the given haystack, returning
the matches as an array-like table of the matching strings.

```lua
local re = kumo.regex.compile '\\b\\w{13}\\b'
local res =
  re:find_all 'Retroactively relinquishing remunerations is reprehensible.'
assert(
  kumo.json_encode(res)
    == '["Retroactively","relinquishing","remunerations","reprehensible"]'
)
```

## regex:replace(TEXT, REPLACEMENT)

Replaces the leftmost-first match with the replacement provided.  `$N` and
`$name` in the replacement string are expanded to match capture groups defined
in the regex.

If no match is found, then a copy of the string is returned unchanged.

All instances of `$name` in the replacement text is replaced with the
corresponding capture group name.

`name` may be an integer corresponding to the index of the capture group
(counted by order of opening parenthesis where `0` is the entire match) or it can
be a name (consisting of letters, digits or underscores) corresponding to a
named capture group.

If `name` isn’t a valid capture group (whether the name doesn’t exist or isn’t
a valid index), then it is replaced with the empty string.

The longest possible `name` is used. e.g., `$1a` looks up the capture group
named `1a` and not the capture group at index `1`. To exert more precise
control over the name, use braces, e.g., `${1}a`.

To write a literal `$` use `$$`.

```lua
local re = kumo.regex.compile '[^01]+'
assert(re:replace('1078910', '') == '1010')
```

```lua
local re = kumo.regex.compile '(?P<last>[^,\\s]+),\\s+(?P<first>\\S+)'
assert(
  re:replace('Springsteen, Bruce', '$first $last') == 'Bruce Springsteen'
)
```

## regex:replace_all(TEXT, REPLACEMENT)

Replaces all non-overlapping matches in text with the replacement provided.
This is the same as calling `replacen` with `limit` set to `0`.

See the documentation for `replace` for details on how to access capturing
group matches in the replacement string.

## regex:replacen(TEXT, LIMIT, REPLACEMENT)

Replaces at most limit non-overlapping matches in text with the replacement
provided. If limit is `0`, then all non-overlapping matches are replaced.

See the documentation for `replace` for details on how to access capturing
group matches in the replacement string.

## regex:split(TEXT)

Splits `text` by the regex, returning each delimited string in an array-style
table.

```lua
local re = kumo.regex.compile '[ \\t]+'
local res = re:split 'a b \t  c\td    e'
assert(kumo.json_encode(res) == '["a","b","c","d","e"]')
```

