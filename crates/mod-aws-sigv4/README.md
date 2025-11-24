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

The module is exposed as `kumo.crypto.aws_sign_v4()` in Lua.

### Basic Example

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
```

For full parameter details and additional examples (SNS, SQS, Firehose,
HTTP client integration), see the `kumo.crypto.aws_sign_v4` page in the
KumoMTA reference documentation (`docs/reference/kumo.crypto/aws_sign_v4.md`).
