local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local main =
  kumo.mimepart.new_text_plain 'Hello, I am the main message content'
local attachment =
  kumo.mimepart.new_binary('application/octet-stream', '\xbb\xaa', {
    file_name = 'binary.dat',
  })

local message =
  kumo.mimepart.new_multipart('multipart/mixed', { main, attachment }, 'woot')

utils.assert_eq(
  tostring(message),
  kumo.string.replace(
    [[
Content-Type: multipart/mixed;
	boundary="woot"

--woot
Content-Type: text/plain;
	charset="us-ascii"

Hello, I am the main message content
--woot
Content-Type: application/octet-stream
Content-Transfer-Encoding: base64
Content-Disposition: attachment;
	filename="binary.dat"

u6o=
--woot--
]],
    '\n',
    '\r\n'
  )
)
