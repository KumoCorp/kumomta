local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local root = kumo.mimepart.new_text_plain 'Hello!'
local headers = root.headers

-- Simple untyped assignment of headers
headers:prepend('X-Woot', 'Woot')
headers:prepend('To', '"John Smith" <john.smith@example.com>')
utils.assert_eq(
  tostring(headers),
  'To: "John Smith" <john.smith@example.com>\r\nX-Woot: Woot\r\nContent-Type: text/plain;\r\n\tcharset="us-ascii"\r\n'
)

-- Verifying that iteration works as expected
local all_headers = {}
for hdr in headers:iter() do
  table.insert(all_headers, { hdr.name, hdr.value })
end
utils.assert_eq(all_headers, {
  {
    'To',
    {
      {
        name = 'John Smith',
        address = { local_part = 'john.smith', domain = 'example.com' },
      },
    },
  },
  { 'X-Woot', 'Woot' },
  {
    'Content-Type',
    {
      value = 'text/plain',
      parameters = {
        charset = 'us-ascii',
      },
    },
  },
})

-- Assignment using accessors
headers:set_from 'user@host'
utils.assert_eq(
  headers:from(),
  { { address = { local_part = 'user', domain = 'host' } } }
)

headers:set_reply_to {
  {
    name = 'Some One',
    address = {
      local_part = 'some.one',
      domain = 'example.com',
    },
  },
}
utils.assert_eq(
  headers:get_first_named('Reply-To').raw_value,
  'Some One <some.one@example.com>'
)

headers:set_cc {
  {
    name = 'Other Person',
    address = {
      local_part = 'other.person',
      domain = 'example.com',
    },
  },
  {
    address = {
      local_part = 'just.email.address',
      domain = 'example.com',
    },
  },
}
utils.assert_eq(
  headers:get_first_named('Cc').raw_value,
  'Other Person <other.person@example.com>,\r\n\t<just.email.address@example.com>'
)

-- Group syntax
headers:set_bcc {
  {
    name = 'The A Team',
    entries = {
      {
        name = 'Bodie',
        address = {
          local_part = 'bodie',
          domain = 'example.com',
        },
      },
      {
        address = {
          local_part = 'doyle',
          domain = 'example.com',
        },
      },
      {
        address = {
          local_part = 'tiger',
          domain = 'example.com',
        },
      },
      {
        address = {
          local_part = 'the.jewellery.man',
          domain = 'example.com',
        },
      },
    },
  },
}

utils.assert_eq(
  headers:get_first_named('bcc').raw_value,
  'The A Team:Bodie <bodie@example.com>,\r\n\t<doyle@example.com>,\r\n\t<tiger@example.com>,\r\n\t<the.jewellery.man@example.com>;'
)

headers:set_subject 'very interesting subject'
utils.assert_eq(headers:subject(), 'very interesting subject')

headers:set_message_id '<123@example.com>'
utils.assert_eq(headers:message_id(), '123@example.com')

-- Verify that assignment validates string values
local ok, err =
  pcall(headers.set_message_id, headers, 'missing.angles@example.com')
utils.assert_eq(ok, false)
utils.assert_matches(tostring(err), 'invalid header')

-- Verify that we're mapping the underlying accessor method type to the
-- more lua-friendly wrapper
local ct = headers:content_type()
utils.assert_eq(ct, {
  value = 'text/plain',
  parameters = {
    charset = 'us-ascii',
  },
})

headers:set_content_type {
  value = 'text/html',
  parameters = {
    charset = 'utf-8',
    extra_somethin_something = 'a dash',
  },
}
utils.assert_eq(
  tostring(headers:get_first_named 'Content-type'),
  'Content-Type: text/html;\r\n\tcharset="utf-8";\r\n\textra_somethin_something="a dash"\r\n'
)
