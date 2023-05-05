# `kumo.http.build_client({PARAMS})`

Constructs an HTTP client object.

The client maintains state, including a connection pool, that can be used
across multiple HTTP requests.

`PARAMS` is an object-style table with the follow keys:

* `user_agent` - optional string that will be used to set the `User-Agent` header
  for all requests made by the client

```lua
local response = kumo.http.build_client({}):get('https://example.com/'):send()
print(response:status_code(), response:status_reason())
for k, v in pairs(response:headers()) do
  print('Header', k, v)
end
print(response:text())
```

## Client Methods

The returned client object has the following methods:

### client:get(URL)

Returns a [Request](Request.md) object that has been configured to make a GET
request to the specified URL.  The URL is a string.

### client:post(URL)

Returns a [Request](Request.md) object that has been configured to make a POST
request to the specified URL.  The URL is a string.

### client:put(URL)

Returns a [Request](Request.md) object that has been configured to make a PUT
request to the specified URL.  The URL is a string.

