# iter

```lua
headers:iter(OPTIONAL_NAME)
```

{{since('dev')}}

Produces an iterator over the headers.  If `OPTIONAL_NAME` is omitted, all
headers are iterated in the order in which they appear in the message.
If `OPTIONAL_NAME` is specified, then all headers that equal the name,
case insensitively, will be iterated in the order in which they appear in the message.

## Iterating all headers

```lua
for hdr in headers:iter() do
  print('got header', hdr.name)
end
```

## Iterating just the authentication results headers

```lua
for hdr in headers:iter 'Authentication-Results' do
  print('got auth result', hdr.name, hdr.value)
end
```



