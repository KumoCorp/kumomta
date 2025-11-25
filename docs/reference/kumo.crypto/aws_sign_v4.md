# aws_sign_v4

{{since('dev')}}

```lua
kumo.crypto.aws_sign_v4(PARAMS)
```

Signs an HTTP request using AWS Signature Version 4 (SigV4). This is useful
when integrating KumoMTA with AWS services such as S3, SNS, SQS, Kinesis
Firehose or Lambda directly from Lua.

`PARAMS` is a table with the following fields:

| Parameter       | Type      | Required | Description                                         |
|----------------|-----------|----------|-----------------------------------------------------|
| `access_key`   | KeySource | Yes      | AWS Access Key ID (see [keysource](../keysource.md)) |
| `secret_key`   | KeySource | Yes      | AWS Secret Access Key (see [keysource](../keysource.md)) |
| `region`       | string    | Yes      | AWS region (e.g. `"us-east-1"`)                     |
| `service`      | string    | Yes      | AWS service name (e.g. `"s3"`, `"sns"`, `"sqs"`)    |
| `method`       | string    | Yes      | HTTP method (e.g. `"GET"`, `"POST"`, `"PUT"`)       |
| `uri`          | string    | Yes      | Request URI path (e.g. `"/my-bucket/object"`)       |
| `query_params` | table     | No       | Query string parameters as key/value pairs          |
| `headers`      | table     | No       | HTTP headers to include in the signature            |
| `payload`      | string    | No       | Request body (empty string for `GET` requests)      |
| `timestamp`    | DateTime  | No       | Override timestamp; defaults to current time        |
| `session_token`| string    | No       | Session token for temporary credentials             |

The function returns a table `RESULT` with these fields:

| Field               | Type   | Description                                           |
|---------------------|--------|-------------------------------------------------------|
| `authorization`     | string | Authorization header value (`AWS4-HMAC-SHA256 ...`)  |
| `timestamp`         | string | AWS timestamp (`YYYYMMDD'T'HHMMSS'Z'`)               |
| `signature`         | string | Hex-encoded signature                                |
| `canonical_request` | string | Canonical request string (for debugging)             |
| `string_to_sign`    | string | String-to-sign (for debugging)                       |

## Examples

### S3 GET request

```lua
local kumo = require 'kumo'

local result = kumo.crypto.aws_sign_v4 {
  access_key = {
    key_data = 'AKIAIOSFODNN7EXAMPLE',
  },
  secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
  region = 'us-east-1',
  service = 's3',
  method = 'GET',
  uri = '/my-bucket/my-object.txt',
  query_params = {},
  headers = {
    host = 'my-bucket.s3.amazonaws.com',
  },
  payload = '',
}

http.request {
  url = 'https://my-bucket.s3.amazonaws.com/my-object.txt',
  method = 'GET',
  headers = {
    ['Authorization'] = result.authorization,
    ['X-Amz-Date'] = result.timestamp,
    ['Host'] = 'my-bucket.s3.amazonaws.com',
  },
}
```

### SNS Publish request

```lua
local kumo = require 'kumo'

local body =
  'Action=Publish&Message=Hello&TopicArn=arn:aws:sns:us-east-1:123456789012:MyTopic'

local sig = kumo.crypto.aws_sign_v4 {
  access_key = {
    key_data = 'AKIAIOSFODNN7EXAMPLE',
  },
  secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
  region = 'us-east-1',
  service = 'sns',
  method = 'POST',
  uri = '/',
  headers = {
    host = 'sns.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-www-form-urlencoded',
  },
  payload = body,
}
```

### Kinesis PutRecord request

```lua
local kumo = require 'kumo'
local http = require 'kumo.http'

local payload = kumo.serde.json_encode {
  StreamName = 'my-stream',
  PartitionKey = 'example-partition',
  Data = kumo.encode.base64_encode 'Hello from KumoMTA',
}

local sig = kumo.crypto.aws_sign_v4 {
  access_key = {
    key_data = os.getenv 'AWS_ACCESS_KEY_ID',
  },
  secret_key = {
    key_data = os.getenv 'AWS_SECRET_ACCESS_KEY',
  },
  region = 'us-east-1',
  service = 'kinesis',
  method = 'POST',
  uri = '/',
  headers = {
    host = 'kinesis.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-amz-json-1.1',
    ['x-amz-target'] = 'Kinesis_20131202.PutRecord',
  },
  payload = payload,
}

local resp = http.request {
  url = 'https://kinesis.us-east-1.amazonaws.com/',
  method = 'POST',
  headers = {
    ['Authorization'] = sig.authorization,
    ['X-Amz-Date'] = sig.timestamp,
    ['X-Amz-Target'] = 'Kinesis_20131202.PutRecord',
    ['Content-Type'] = 'application/x-amz-json-1.1',
    ['Host'] = 'kinesis.us-east-1.amazonaws.com',
  },
  body = payload,
}
```

### Lambda Invoke request

```lua
local kumo = require 'kumo'
local http = require 'kumo.http'

local invoke_payload = kumo.serde.json_encode {
  key = 'value',
}

local function_name = 'my-function'

local sig = kumo.crypto.aws_sign_v4 {
  access_key = {
    key_data = os.getenv 'AWS_ACCESS_KEY_ID',
  },
  secret_key = {
    key_data = os.getenv 'AWS_SECRET_ACCESS_KEY',
  },
  region = 'us-east-1',
  service = 'lambda',
  method = 'POST',
  uri = ('/2015-03-31/functions/%s/invocations'):format(function_name),
  headers = {
    host = 'lambda.us-east-1.amazonaws.com',
    ['content-type'] = 'application/json',
  },
  payload = invoke_payload,
}

local resp = http.request {
  url = ('https://lambda.us-east-1.amazonaws.com/2015-03-31/functions/%s/invocations'):format(
    function_name
  ),
  method = 'POST',
  headers = {
    ['Authorization'] = sig.authorization,
    ['X-Amz-Date'] = sig.timestamp,
    ['Host'] = 'lambda.us-east-1.amazonaws.com',
    ['Content-Type'] = 'application/json',
  },
  body = invoke_payload,
}
```
