local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'
print 'doing mimepart tests'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

local BASIC_CONTENT = 'From: me@example.com\r\nSubject: hello\r\n\r\nHi'

-- Try parsing via a Message object
local msg = new_msg(BASIC_CONTENT)
local mime = msg:parse_mime()

-- Confirm that parsing directly into a MimePart is consistent
local alt_mime = kumo.mimepart.parse(BASIC_CONTENT)
utils.assert_eq(tostring(mime), tostring(alt_mime))

-- Examine the simple structure
local structure = mime:get_simple_structure()
utils.assert_eq(structure.html, nil)

utils.assert_eq(structure.text_part.body, 'Hi')
structure.text_part.body = 'hello world!\r\n'
utils.assert_eq(structure.text_part.body, 'hello world!\r\n')

local headers = {}
for hdr in mime.headers:iter() do
  headers[hdr.name] = hdr.unstructured
end

utils.assert_eq(headers, {
  ['Content-Type'] = 'text/plain; charset="us-ascii"',
  From = 'me@example.com',
  Subject = 'hello',
})

local subject = mime.headers:get_first_named 'subject'
utils.assert_eq(subject.unstructured, 'hello')

utils.assert_eq(
  tostring(structure.text_part),
  'From: me@example.com\r\nSubject: hello\r\nContent-Type: text/plain;\r\n\tcharset="us-ascii"\r\n\r\nhello world!\r\n'
)

-- structure.text_part should behave like an alias to mime (the parsed root),
-- so we should observe the mutation there too
utils.assert_eq(
  tostring(mime),
  'From: me@example.com\r\nSubject: hello\r\nContent-Type: text/plain;\r\n\tcharset="us-ascii"\r\n\r\nhello world!\r\n'
)
