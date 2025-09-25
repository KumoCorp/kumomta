local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

-- Build up an email with a couple of attachments/parts
local builder = kumo.mimepart.builder()
builder:set_stable_content(true)
builder:text_plain 'Hello, plain'
builder:text_html '<b>Hello, html</b>'
builder:attach(
  'text/plain',
  'I am a plain text file with no options specified'
)
builder:attach('application/octet-stream', '\xaa\xbb', {
  file_name = 'binary.dat',
})
local root = builder:build()
-- print(root)

-- Now interpret the message structure and extract the attachments
local structure = root:get_simple_structure()
local attachments = {}
for _, att in ipairs(structure.attachments) do
  table.insert(
    attachments,
    { att.file_name, att.content_type, att.part.body }
  )
end

utils.assert_eq(attachments, {
  {
    nil,
    'text/plain',
    'I am a plain text file with no options specified',
  },
  { 'binary.dat', 'application/octet-stream', '\xaa\xbb' },
})
