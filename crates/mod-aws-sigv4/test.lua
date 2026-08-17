-- AWS SigV4 Signing Test Example
-- This demonstrates how to use the AWS SigV4 module to sign requests

local kumo = require 'kumo'
local crypto = kumo.crypto
local utils = require 'policy-extras.policy_utils'

-- Fixed timestamp so that signatures are deterministic for testing.
local FIXED_TIME = '2025-11-21T20:04:15Z'

-- Example 1: Basic S3 GET request signature.
-- S3 requires x-amz-content-sha256 to be signed.
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
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=b438a29cae8b5d9fd3634e7b70ea4fdedb1fea96260aae6a9cf2e6b0ad5a4028'
)

-- Example 2: SNS POST. Non-S3 services do not auto-add x-amz-content-sha256.
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
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/sns/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=2b3db398c42cb0d2476ff7d66f97406a9d276d1404e219e2e6e3be8606a981ef'
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
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/sqs/aws4_request, SignedHeaders=host;x-amz-date, Signature=4551d656b1ae2d3e2468ae3dad8ec2cba0fb489920e1260a4c7f64189a8f814d'
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
  'AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20251121/us-east-1/firehose/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amz-target, Signature=1d743f74da9abd6399416b5871a74ac3453766ccdb3d60b481e45a8b19549851'
)
