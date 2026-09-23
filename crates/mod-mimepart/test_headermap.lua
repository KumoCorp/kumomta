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
    name = 'Test [hello]',
    address = {
      local_part = 'some.one',
      domain = 'example.com',
    },
  },
}
utils.assert_eq(
  headers:get_first_named('Reply-To').raw_value,
  '"Test [hello]" <some.one@example.com>'
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
  '"Other Person" <other.person@example.com>,\r\n\t<just.email.address@example.com>'
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

-- Verify that we hard-wrap excessively long lines
local long_string = string.rep('A', 1500)
local expect_wrapped = string.rep('A', 900)
  .. '\r\n\t'
  .. string.rep('A', 600)
headers:prepend('X-Long', long_string)
utils.assert_eq(
  tostring(headers:get_first_named 'X-Long'),
  'X-Long: ' .. expect_wrapped .. '\r\n'
)

-- Soft wrap lines with spaces
local long_string = string.rep('hello there ', 10)
headers:prepend('X-Long', long_string)
utils.assert_eq(
  tostring(headers:get_first_named 'X-Long'),
  'X-Long: hello there hello there hello there hello there hello there hello there\r\n'
    .. '\thello there hello there hello there hello there\r\n'
)

-- Verify `header.value` for each header grammar now that it routes through
-- ParsedHeader.
local vheaders = kumo.mimepart.new_text_plain('x').headers
local function header_value(name, raw)
  vheaders:prepend(name, raw)
  return vheaders:get_first_named(name).value
end

utils.assert_eq(header_value('From', 'Someone <someone@example.com>'), {
  {
    name = 'Someone',
    address = { local_part = 'someone', domain = 'example.com' },
  },
})
utils.assert_eq(
  header_value('Sender', 'Someone <someone@example.com>'),
  {
    name = 'Someone',
    address = { local_part = 'someone', domain = 'example.com' },
  }
)
utils.assert_eq(header_value('To', '"John Smith" <john@example.com>'), {
  {
    name = 'John Smith',
    address = { local_part = 'john', domain = 'example.com' },
  },
})
utils.assert_eq(
  header_value('Message-ID', '<123@example.com>'),
  '123@example.com'
)
utils.assert_eq(
  header_value('Content-ID', '<abc@example.com>'),
  'abc@example.com'
)
utils.assert_eq(
  header_value('References', '<a@example.com> <b@example.com>'),
  { 'a@example.com', 'b@example.com' }
)
utils.assert_eq(
  header_value('Content-Transfer-Encoding', 'quoted-printable'),
  { value = 'quoted-printable', parameters = {} }
)
utils.assert_eq(
  header_value('Content-Disposition', 'attachment; filename="x.txt"'),
  { value = 'attachment', parameters = { filename = 'x.txt' } }
)
utils.assert_eq(
  header_value(
    'Authentication-Results',
    'example.com; dkim=pass header.d=example.com'
  ),
  {
    serv_id = 'example.com',
    results = {
      {
        props = { ['header.d'] = 'example.com' },
        result = 'pass',
        method = 'dkim',
      },
    },
  }
)
utils.assert_eq(header_value('Subject', 'hello there'), 'hello there')
utils.assert_eq(header_value('X-Custom', 'whatever'), 'whatever')

-- Date is deliberately exempted from parsing (see header_value_to_lua): its
-- value stays the raw string, which should be an RFC 2822 compatible date.
utils.assert_eq(
  header_value('Date', 'Tue, 1 Jul 2003 10:52:37 +0200'),
  'Tue, 1 Jul 2003 10:52:37 +0200'
)
