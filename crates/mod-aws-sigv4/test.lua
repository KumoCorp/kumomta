-- AWS SigV4 Signing Test Example
-- This demonstrates how to use the AWS SigV4 module to sign requests

local kumo = require 'kumo'
local mod = kumo.aws

-- Example 1: Basic S3 GET request signature
print '=== Example 1: S3 GET Request ==='

local result = mod.sign_v4 {
  access_key = 'AKIAIOSFODNN7EXAMPLE',
  secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
  region = 'us-east-1',
  service = 's3',
  method = 'GET',
  uri = '/test.txt',
  query_params = {},
  headers = {
    host = 'examplebucket.s3.amazonaws.com',
  },
  payload = '',
}

print('Authorization Header: ' .. result.authorization)
print('Timestamp: ' .. result.timestamp)
print('Signature: ' .. result.signature)
print()

-- Example 2: POST request with payload
print '=== Example 2: SNS POST Request ==='

local sns_payload =
  'Action=Publish&Message=test&TopicArn=arn:aws:sns:us-east-1:123456789012:test'

local result2 = mod.sign_v4 {
  access_key = 'AKIAIOSFODNN7EXAMPLE',
  secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
  region = 'us-east-1',
  service = 'sns',
  method = 'POST',
  uri = '/',
  query_params = {},
  headers = {
    host = 'sns.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-www-form-urlencoded',
  },
  payload = sns_payload,
}

print('Authorization Header: ' .. result2.authorization)
print('Timestamp: ' .. result2.timestamp)
print()

-- Example 3: SQS request with query parameters
print '=== Example 3: SQS Request with Query Parameters ==='

local result3 = mod.sign_v4 {
  access_key = 'AKIAIOSFODNN7EXAMPLE',
  secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
  region = 'us-east-1',
  service = 'sqs',
  method = 'GET',
  uri = '/',
  query_params = {
    Action = 'SendMessage',
    MessageBody = 'Hello World',
    QueueUrl = 'https://sqs.us-east-1.amazonaws.com/123456789012/MyQueue',
  },
  headers = {
    host = 'sqs.us-east-1.amazonaws.com',
  },
  payload = '',
}

print('Authorization Header: ' .. result3.authorization)
print('Signature: ' .. result3.signature)
print()

-- Example 4: Using external key source (environment variable or file)
print '=== Example 4: Using KeySource ==='

-- You can load keys from environment variables or files
local result4 = mod.sign_v4 {
  access_key = 'AKIAIOSFODNN7EXAMPLE',
  secret_key = {
    -- Load from file (pass string directly)
    -- '/path/to/secret/key'
    -- Or inline value using key_data
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
  region = 'us-west-2',
  service = 's3',
  method = 'PUT',
  uri = '/my-bucket/my-object.txt',
  query_params = {},
  headers = {
    host = 'my-bucket.s3.us-west-2.amazonaws.com',
  },
  payload = 'This is the content of my file',
}

print('Authorization Header: ' .. result4.authorization)
print()

-- Example 5: Kinesis Firehose PutRecord
print '=== Example 5: Kinesis Firehose PutRecord ==='

-- Kinesis Firehose payload (JSON)
local firehose_payload = [[{
"DeliveryStreamName": "my-delivery-stream",
"Record": {
    "Data": "SGVsbG8gZnJvbSBLdW1vTVRBIQ=="
}
}]]

local result5 = mod.sign_v4 {
  access_key = 'AKIAIOSFODNN7EXAMPLE',
  secret_key = {
    key_data = 'wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY',
  },
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

print('Authorization Header: ' .. result5.authorization)
print('Timestamp: ' .. result5.timestamp)
print('Signature: ' .. result5.signature)
print()
print 'Example HTTP request headers to use with Kinesis Firehose:'
print('  Authorization: ' .. result5.authorization)
print('  X-Amz-Date: ' .. result5.timestamp)
print '  X-Amz-Target: Firehose_20150804.PutRecord'
print '  Content-Type: application/x-amz-json-1.1'
print '  Host: firehose.us-east-1.amazonaws.com'
print()

print '=== Debugging Information ==='
print 'Canonical Request:'
print(result.canonical_request)
print()
print 'String to Sign:'
print(result.string_to_sign)
