-- AWS SigV4 Signing Test Example
-- This demonstrates how to use the AWS SigV4 module to sign requests

local kumo = require 'kumo'
local crypto = kumo.crypto
local utils = require 'policy-extras.policy_utils'

-- Fixed timestamp so that signatures are deterministic for testing.
local FIXED_TIME = '2025-11-21T20:04:15Z'

-- Example 1: Basic S3 GET request signature
local result = crypto.aws_sign_v4 {
  access_key = {
    key_data = 'AKIAIOSFODNN7EXAMPLE',
  },
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
  timestamp = FIXED_TIME,
}

utils.assert_eq(
  result.authorization,
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-date, Signature=4b59c9035eb4883857b0be8815678ee9a00179e4f4ac98b8ec79237e2d41dc4b'
)

local sns_payload =
  'Action=Publish&Message=test&TopicArn=arn:aws:sns:us-east-1:123456789012:test'

local result2 = crypto.aws_sign_v4 {
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
  query_params = {},
  headers = {
    host = 'sns.us-east-1.amazonaws.com',
    ['content-type'] = 'application/x-www-form-urlencoded',
  },
  payload = sns_payload,
  timestamp = FIXED_TIME,
}

utils.assert_eq(
  result2.authorization,
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/sns/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date, Signature=6b5274220a51821b2293cde3ad7317e2b8dda14240e29de8e63e8f81869e4f91'
)

-- Example 3: SQS request with query parameters
local result3 = crypto.aws_sign_v4 {
  access_key = {
    key_data = 'AKIAIOSFODNN7EXAMPLE',
  },
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
  timestamp = FIXED_TIME,
}

utils.assert_eq(
  result3.authorization,
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/sqs/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=27ccd47c88383aa24294feb1adaad19fffd4f49657be40599d52425f2179c416'
)

-- Example 4: Using external key source (KeySource)
local result4 = crypto.aws_sign_v4 {
  access_key = {
    key_data = 'AKIAIOSFODNN7EXAMPLE',
  },
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
  timestamp = FIXED_TIME,
}

-- We don't assert a specific signature here; this primarily exercises
-- the KeySource handling and non-GET methods.
utils.assert_eq(
  result4.timestamp,
  FIXED_TIME:gsub('%-', ''):gsub(':', ''):gsub('T', 'T'):gsub('Z', 'Z')
)

-- Example 5: Kinesis Firehose PutRecord

-- Kinesis Firehose payload (JSON)
local firehose_payload = [[{
"DeliveryStreamName": "my-delivery-stream",
"Record": {
    "Data": "SGVsbG8gZnJvbSBLdW1vTVRBIQ=="
}
}]]

local result5 = crypto.aws_sign_v4 {
  access_key = {
    key_data = 'AKIAIOSFODNN7EXAMPLE',
  },
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
  timestamp = FIXED_TIME,
}

utils.assert_eq(
  result5.authorization,
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/firehose/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;x-amz-target, Signature=e1d98cf03e5cbd7abd909cce023c05cfd8ff51923c96d76798e261ae67307752'
)
