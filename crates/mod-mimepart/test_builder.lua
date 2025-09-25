local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- Try the builder API
local builder = kumo.mimepart.builder()
builder:text_plain 'Hello there!'
builder:text_html '<b>Hello</b>'
builder:set_stable_content(true)
local root = builder:build()
utils.assert_eq(
  tostring(root),
  'Content-Type: multipart/alternative;\r\n\tboundary="ma-boundary"\r\nMime-Version: 1.0\r\nDate: Tue, 1 Jul 2003 10:52:37 +0200\r\n\r\n--ma-boundary\r\nContent-Type: text/plain;\r\n\tcharset="us-ascii"\r\n\r\nHello there!\r\n--ma-boundary\r\nContent-Type: text/html;\r\n\tcharset="us-ascii"\r\n\r\n<b>Hello</b>\r\n--ma-boundary--\r\n'
)
