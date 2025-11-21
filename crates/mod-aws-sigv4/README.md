# AWS SigV4 Module for KumoMTA

This module provides AWS Signature Version 4 (SigV4) signing functionality for KumoMTA. It allows you to sign HTTP requests to AWS services using the AWS SigV4 authentication protocol.

## Features

- Full AWS SigV4 signature implementation
- Support for all AWS services (S3, SNS, SQS, Kinesis Firehose, etc.)
- Query parameter signing
- Header signing
- Payload hashing (SHA256)
- Secure key management via KeySource (files, Vault, inline, or events)
- Session token support for temporary credentials
- Comprehensive debugging output

## Usage

The module is exposed as `kumo.aws.sign_v4()` in Lua.

### Basic Example

```lua
local kumo = require 'kumo'

local result = kumo.aws.sign_v4 {
-- AWS credentials
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
},

-- AWS region and service
region = 'us-east-1',
service = 's3',

-- HTTP request details
method = 'GET',
uri = '/my-bucket/my-object.txt',
query_params = {},
headers = {
    host = 'my-bucket.s3.amazonaws.com',
},
payload = '',
}

-- Use the result
print('Authorization: ' .. result.authorization)
print('X-Amz-Date: ' .. result.timestamp)
```

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `access_key` | string | Yes | AWS Access Key ID |
| `secret_key` | KeySource | Yes | AWS Secret Access Key (see KeySource below) |
| `region` | string | Yes | AWS region (e.g., "us-east-1") |
| `service` | string | Yes | AWS service name (e.g., "s3", "sns", "sqs") |
| `method` | string | Yes | HTTP method (e.g., "GET", "POST", "PUT") |
| `uri` | string | Yes | Request URI path (e.g., "/my-bucket/object") |
| `query_params` | table | No | Query string parameters as key-value pairs |
| `headers` | table | No | HTTP headers to include in signature |
| `payload` | string | No | Request body (empty string for GET requests) |
| `timestamp` | DateTime | No | Override timestamp (defaults to current time) |
| `session_token` | string | No | Session token for temporary credentials |

### KeySource Options

The `secret_key` parameter supports multiple methods for providing the secret:

```lua
-- Option 1: Inline value (for testing/development)
secret_key = {
key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
}

-- Option 2: Load from file (recommended for production)
secret_key = '/path/to/secret/key/file'

-- Option 3: Load from Vault (most secure)
secret_key = {
vault_mount = 'secret',
vault_path = 'aws/credentials',
vault_key = 'secret_key',
}
```

### Return Value

The function returns a table with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `authorization` | string | Complete Authorization header value |
| `timestamp` | string | Timestamp in AWS format (YYYYMMDD'T'HHMMSS'Z') |
| `signature` | string | The computed signature (hex encoded) |
| `canonical_request` | string | The canonical request (for debugging) |
| `string_to_sign` | string | The string that was signed (for debugging) |

## Examples

### S3 GET Request

```lua
local kumo = require 'kumo'

local result = kumo.aws.sign_v4 {
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = { key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY' },
region = 'us-east-1',
service = 's3',
method = 'GET',
uri = '/my-bucket/my-file.txt',
query_params = {},
headers = { host = 'my-bucket.s3.amazonaws.com' },
payload = '',
}
```

### S3 PUT Request with Payload

```lua
local kumo = require 'kumo'
local content = 'Hello, World!'

local result = kumo.aws.sign_v4 {
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = { key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY' },
region = 'us-west-2',
service = 's3',
method = 'PUT',
uri = '/my-bucket/greeting.txt',
query_params = {},
headers = {
    host = 'my-bucket.s3.us-west-2.amazonaws.com',
    ['content-type'] = 'text/plain',
},
payload = content,
}
```

### SNS Publish Request

```lua
local kumo = require 'kumo'
local message_body = 'Action=Publish&Message=Hello&TopicArn=arn:aws:sns:us-east-1:123456789012:MyTopic'

local result = kumo.aws.sign_v4 {
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = { key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY' },
region = 'us-east-1',
service = 'sns',
method = 'POST',
uri = '/',
query_params = {},
headers = {
    host = 'sns.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-www-form-urlencoded',
},
payload = message_body,
}
```

### SQS with Query Parameters

```lua
local kumo = require 'kumo'

local result = kumo.aws.sign_v4 {
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = { key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY' },
region = 'us-east-1',
service = 'sqs',
method = 'GET',
uri = '/',
query_params = {
    Action = 'SendMessage',
    MessageBody = 'Hello from KumoMTA',
    QueueUrl = 'https://sqs.us-east-1.amazonaws.com/123456789012/MyQueue',
},
headers = { host = 'sqs.us-east-1.amazonaws.com' },
payload = '',
}
```

### Kinesis Firehose PutRecord

```lua
local kumo = require 'kumo'

-- JSON payload for PutRecord
local firehose_payload = [[{
"DeliveryStreamName": "my-delivery-stream",
"Record": {
    "Data": "SGVsbG8gZnJvbSBLdW1vTVRBIQ=="
}
}]]

local result = kumo.aws.sign_v4 {
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = { key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY' },
region = 'us-east-1',
service = 'firehose',
method = 'POST',
uri = '/',
query_params = {},
headers = {
    host = 'firehose.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-amz-json-1.1',
    ['x-amz-target'] = 'Firehose_20150804.PutRecord',
},
payload = firehose_payload,
}
```

### Using with HTTP Client

Here's a complete example of using the signed request with KumoMTA's HTTP client:

```lua
local kumo = require 'kumo'
local http = require 'kumo.http'

-- Sign the request
local signature = kumo.aws.sign_v4 {
access_key = 'AKIAIOSFODNN7EXAMPLE',
secret_key = '/path/to/secret/key/file',  -- Load from file in production
region = 'us-east-1',
service = 's3',
method = 'GET',
uri = '/my-bucket/data.json',
query_params = {},
headers = { host = 'my-bucket.s3.amazonaws.com' },
payload = '',
}

-- Make the HTTP request
local response = http.request {
url = 'https://my-bucket.s3.amazonaws.com/data.json',
method = 'GET',
headers = {
    ['Authorization'] = signature.authorization,
    ['X-Amz-Date'] = signature.timestamp,
    ['Host'] = 'my-bucket.s3.amazonaws.com',
},
}

print('Status: ' .. response.status)
print('Body: ' .. response.body)
```

### Complete Kinesis Firehose Integration

Here's a complete example sending data to Kinesis Firehose:

```lua
local kumo = require 'kumo'
local http = require 'kumo.http'

-- Prepare the Firehose payload
local firehose_payload = kumo.serde.json_encode {
DeliveryStreamName = 'my-delivery-stream',
Record = {
    Data = kumo.encode.base64_encode('Email delivery log: recipient@example.com delivered'),
},
}

-- Sign the request
local signature = kumo.aws.sign_v4 {
access_key = os.getenv('AWS_ACCESS_KEY_ID') or 'AKIAIOSFODNN7EXAMPLE',
secret_key = '/path/to/secret/key/file',  -- Or use Vault in production
region = 'us-east-1',
service = 'firehose',
method = 'POST',
uri = '/',
query_params = {},
headers = {
    host = 'firehose.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-amz-json-1.1',
    ['x-amz-target'] = 'Firehose_20150804.PutRecord',
},
payload = firehose_payload,
}

-- Send the request to Firehose
local response = http.request {
url = 'https://firehose.us-east-1.amazonaws.com/',
method = 'POST',
headers = {
    ['Authorization'] = signature.authorization,
    ['X-Amz-Date'] = signature.timestamp,
    ['X-Amz-Target'] = 'Firehose_20150804.PutRecord',
    ['Content-Type'] = 'application/x-amz-json-1.1',
    ['Host'] = 'firehose.us-east-1.amazonaws.com',
},
body = firehose_payload,
}

if response.status == 200 then
print('Successfully sent to Firehose!')
local result = kumo.serde.json_decode(response.body)
print('Record ID: ' .. result.RecordId)
else
print('Error: ' .. response.status)
print('Body: ' .. response.body)
end
```

## AWS Signature V4 Process

The module implements the complete AWS SigV4 signing process:

1. **Create Canonical Request**: Normalizes the HTTP request components
- HTTP method
- Canonical URI (percent-encoded path)
- Canonical query string (sorted, percent-encoded parameters)
- Canonical headers (lowercase, trimmed, sorted)
- Signed headers (list of headers included in signature)
- Payload hash (SHA256 of request body)

2. **Create String to Sign**: Combines timestamp, credential scope, and canonical request hash
- Algorithm identifier: `AWS4-HMAC-SHA256`
- Request timestamp
- Credential scope: `YYYYMMDD/region/service/aws4_request`
- Hash of canonical request

3. **Calculate Signing Key**: Derives the signing key from secret key
- Uses HMAC-SHA256 with date, region, service
- Final key is used only for this signature

4. **Calculate Signature**: Signs the string to sign with the signing key
- HMAC-SHA256 of string to sign
- Result is hex-encoded

5. **Create Authorization Header**: Formats the complete authorization value
- `AWS4-HMAC-SHA256 Credential=..., SignedHeaders=..., Signature=...`

## Security Best Practices

1. **Never hardcode credentials** in your policy files
2. **Use KeySource** with file paths, Vault, or events for production
3. **Rotate credentials** regularly
4. **Use IAM roles** when running in AWS environments
5. **Use temporary credentials** (session tokens) when possible
6. **Restrict IAM permissions** to minimum required

## Debugging

The module returns debugging information that can help troubleshoot signature issues:

```lua
local kumo = require 'kumo'
local result = kumo.aws.sign_v4 { ... }

print('Canonical Request:')
print(result.canonical_request)
print()
print('String to Sign:')
print(result.string_to_sign)
print()
print('Signature:')
print(result.signature)
```

Compare these values with AWS's signature calculator or request logs to identify issues.

## Testing

Run the included test file:

```bash
kumod --script --policy crates/mod-aws-sigv4/test.lua
```

## References

- [AWS Signature Version 4 Documentation](https://docs.aws.amazon.com/general/latest/gr/signature-version-4.html)
- [AWS Signature V4 Signing Process](https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html)
