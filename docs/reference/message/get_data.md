# get_data

```lua
message:get_data()
```

Returns the message body/data as a string.  This is typically the full email
message and headers, but for log messages this will typically be the
json-encoded representaiton of the log record.

See also:
* [msg:set_data()](set_data.md)
