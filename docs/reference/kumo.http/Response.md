# The HTTP Response Object

This object is returned from [request:send()](Request.md#requestsend) and represents
the in-progress response.  Status and header information is available to query from
the response object ahead of requesting the body in either text or byte format.
Once the body has been retrieved, the other methods of the response object can
no longer be called.

The following methods are supported:

## response:status_code()

Returns the numeric HTTP status code indicating the outcome of the request.

## response:status_reason()

Returns the standardized reason-phrase representation of the status code. May return nil
if the status code is non-standard or otherwise unknown.

The purpose of the reason is for human understanding and logging. You should not use
the reason in conditional logic.

## response:status_is_informational()

Returns true if the status code is in the range `100` - `199`.

## response:status_is_success()

Returns true if the status code is in the range `200` - `299`.

## response:status_is_redirection()

Returns true if the status code is in the range `300` - `399`.

## response:status_is_client_error()

Returns true if the status code is in the range `400` - `499`.

## response:status_is_server_error()

Returns true if the status code is in the range `500` - `599`.

## response:headers()

Returns a headermap object that holds the response headers from the request.

You can index the headers to look up a specific result:

```lua
local headers = response:headers()
print('x-header value is', headers['x-header'])
```

Indexing into the headermap is case-insensitive, so these are both equivalent:

```lua
local headers = response:headers()
print('x-header value is', headers['x-header'])
print('x-header value is', headers['X-Header'])
```

You can also iterate over the headers:

```lua
local headers = response:headers()
for k, v in pairs(headers) do
  print('header', k, v)
end
```

## response:content_length()

Returns the length of the response data in bytes, or nil if no `Content-Length`
header was present in the response.

Note that this should be the length of the data returned by the `response:bytes()` method,
but because the `response:text()` method may perform conversion to produce UTF-8, it may
not be the same as the length of the textual result.

## response:bytes()

Returns the raw response content as bytes.

## response:text()

This method decodes the response body with BOM sniffing and with malformed
sequences replaced with the REPLACEMENT CHARACTER. Encoding is determined from
the `charset` parameter of the `Content-Type` header, and defaults to utf-8 if
not presented.

