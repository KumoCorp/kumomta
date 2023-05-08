# The HTTP Request Object

This object is returned from [client:get()](build_client.md#clientgeturl),
[client:post()](build_client.md#clientposturl) and
[client:put()](build_client.md#clientputurl) and represents a request that
has yet to be sent to the remote server.

You can use the methods of this object to further configure the request,
and then send the request.

The following methods are supported:

## request:header(name, value)

Sets an HTTP header.  `name` and `value` are both strings and correspond to the
header name and value respectively.

```lua
request:header('Content-Type', 'application/json')
```

## request:headers({HEADERS})

Sets multiple HTTP headers. `HEADERS` is an object-style table holding
name/value pairs for the headers and values that should be set.

```lua
request:headers {
  ['Content-Type'] = 'application/json',
  ['X-Something'] = 'value',
}
```

## request:basic_auth(username \[, password\])

Configures the username and optional password that should be used
to perform HTTP Basic authentication.

## request:bearer_auth(token)

Configures the token to be used for HTTP Bearer authentication

## request:body(body)

Sets the body of the request. Body must be a string.

```lua
local request = kumo.http.build_client({}):post 'https://example.com'
request:header('Content-Type', 'application/json')
request:body(kumo.json_encode {
  key = 'value',
})
```

## request:form_url_encoded({PARAMS})

Sets the body of the request to the provided parameters, using the
`application/x-www-form-urlencoded` encoding scheme. The `Content-Type` header
is implicitly set to `application/x-www-form-urlencoded`.

`PARAMS` is an object-style table whose values must be UTF-8 strings.

```lua
local request = kumo.http.build_client({}):post 'https://example.com'
request:form_url_encoded({
  key = 'value',
  other_key = 'other_value'
})
```

## request:form_multipart_data({PARAMS})

Sets the body of the request to the provided parameters, using the
`multipart/form-data` encoding scheme. The `Content-Type` header
is implicitly set to `multipart/form-data` with the automatically
determined boundary field.

`PARAMS` is an object-style table whose values should be either
UTF-8 strings or lua binary strings.  Binary strings are encoded
as `application/octet-stream` in the generated form data.

```lua
local request = kumo.http.build_client({}):post 'https://example.com'
request:form_multipart_data({
  key = 'value',
  other_key = 'other_value'
})
```

## request:send()

Sends the request and returns [Response](Response.md) object representing
the result of the request.

