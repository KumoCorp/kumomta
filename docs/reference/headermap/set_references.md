# set_references

```lua
headers:set_references(VALUE)
```

{{since('dev')}}

Assign `VALUE` to the `References` header.

`VALUE` may be either a string or an array style table of Message-Id strings.

If you assign using a string, the string will be parsed and validated as being
compatible with the `References` header before allowing the assigment to proceed.

