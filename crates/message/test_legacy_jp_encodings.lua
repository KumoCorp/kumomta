local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

local JP_UTF8 =
  '日本語のテストメールです。こんにちは、ありがとうございます。'
local SHIFT_JIS = kumo.encode.charset_encode('shift-jis', JP_UTF8)
utils.assert_eq(kumo.encode.charset_decode('shift-jis', SHIFT_JIS), JP_UTF8)

local BODY = 'From: someone@example.com\r\nSubject: Testing\r\n\r\n'
  .. SHIFT_JIS
local msg = new_msg(BODY)

local mime_before = msg:parse_mime()
-- We might hope that we get the SHIFT_JIS bytes back out for the body,
-- but we'll try to interpret it as UTF-8 for internationalize email's sake,
-- and this input just happens to look like plausible UTF-8, even though
-- it is complete nonsense
utils.assert_ne(mime_before.body, SHIFT_JIS)

-- Interesting to note that the charset-normalizer based utf-8 decoder
-- won't allow converting these shift-jis bytes to utf-8, but Rust's
-- own built in str::from_utf8 will.
-- utils.assert_eq(mime_before.body, kumo.encode.charset_decode('utf-8', SHIFT_JIS))

-- Can we dkim-sign this binary content?
local signer = kumo.dkim.rsa_sha256_signer {
  domain = 'example.com',
  selector = 'default',
  headers = { 'From', 'Subject' },
  key = 'example-private-dkim-key.pem',
}
msg:dkim_sign(signer)
-- kumo.log_info(msg:get_data())

-- Use encoding detection to fix the encoding up. Note that this
-- definitely breaks the dkim signature, but we don't care in the
-- context of this test; the above is testing whether can sign
-- the wonky encoding, the below is testing whether we can fix
-- it.  In practice, a given node will be doing either one or
-- the other.
msg:check_fix_conformance('', 'NEEDS_TRANSFER_ENCODING', {
  detect_encoding = true,
  include_encodings = {
    'shift-jis',
  },
  exclude_encodings = {},
})

local mime = msg:parse_mime()
utils.assert_eq(mime.body, JP_UTF8)
