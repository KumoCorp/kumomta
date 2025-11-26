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

Consider using a [keysource](../keysource.md) with
[kumo.secrets.load](../kumo.secrets/load.md) to retrieve credentials.

## request:bearer_auth(token)

Configures the token to be used for HTTP Bearer authentication

Consider using a [keysource](../keysource.md) with
[kumo.secrets.load](../kumo.secrets/load.md) to retrieve credentials.

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
request:form_url_encoded {
  key = 'value',
  other_key = 'other_value',
}
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
request:form_multipart_data {
  key = 'value',
  other_key = 'other_value',
}
```

## request:send()

Sends the request and returns [Response](Response.md) object representing
the result of the request.

## request:timeout(duration)

{{since('2024.06.10-84e84b89', indent=True)}}
    Sets the timeout duration for the request.  If no response is received
    within the specified duration, the request will raise an error.

    The default timeout is `"1 minute"`.

    You may pass a duration string like `"1 minute"`.

## request:aws_sign_v4({PARAMS})

{{since('dev', indent=True)}}
    Signs this request using AWS Signature Version 4 (SigV4).

    This is a convenience wrapper around
    [kumo.crypto.aws_sign_v4](../kumo.crypto/aws_sign_v4.md) that derives
    the HTTP method, URI path and query parameters from the `Request`
    object, and then applies the resulting `Authorization` and `X-Amz-Date`
    headers back onto the same request.

    `PARAMS` is a table with the following fields (a subset of the
    `kumo.crypto.aws_sign_v4` parameters):

    - `access_key` (KeySource, required): AWS access key id
    - `secret_key` (KeySource, required): AWS secret access key
    - `region` (string, required): AWS region such as `"us-east-1"`
    - `service` (string, required): AWS service name such as `"s3"`, `"sns"`,
      `"sqs"`, `"firehose"`, `"lambda"`, `"kinesis"`, and so on.
    - `headers` (table, optional): additional headers to include in the
      signature; these are merged with a `host` header derived from the
      request URL if not already present.
    - `payload` (string, optional): request body to use when computing the
      payload hash. When present, this should match the body that is sent
      with the request.
    - `timestamp` (DateTime, optional): override the signing timestamp; if
      omitted, the current time is used.
    - `session_token` (string, optional): session token for temporary
      credentials.

    The method does not automatically read or clone the request body; if
    you are sending a body and need it to be part of the signature, pass
    the same value in the `payload` field.

    ```lua
    local http = require 'kumo.http'

    local client = http.build_client {}
    local req = client:post 'https://kinesis.us-east-1.amazonaws.com/'

    local payload = kumo.serde.json_encode {
      StreamName = 'my-stream',
      PartitionKey = 'example-partition',
      Data = kumo.encode.base64_encode 'Hello from KumoMTA',
    }

    req:header('content-type', 'application/x-amz-json-1.1')
      :header('x-amz-target', 'Kinesis_20131202.PutRecord')
      :body(payload)
      :aws_sign_v4 {
        access_key = { key_data = os.getenv 'AWS_ACCESS_KEY_ID' },
        secret_key = { key_data = os.getenv 'AWS_SECRET_ACCESS_KEY' },
        region = 'us-east-1',
        service = 'kinesis',
        payload = payload,
      }

    local resp = req:send()
    ```
