local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local ft = kumo.file_type.from_bytes '\xCA\xFE\xBA\xBE'
utils.assert_eq(ft, {
  name = 'Java class file',
  extensions = { 'class' },
  media_types = {
    'application/java',
    'application/java-byte-code',
    'application/java-vm',
    'application/x-httpd-java',
    'application/x-java',
    'application/x-java-class',
    'application/x-java-vm',
  },
})

local markdown = kumo.file_type.from_extension 'markdown'
utils.assert_eq(markdown, {
  {
    name = 'Q1193600',
    media_types = {
      'text/markdown',
    },
    extensions = {
      'markdown',
      'md',
      'mdown',
      'mdtext',
      'mdtxt',
      'mkd',
    },
  },
})

local png = kumo.file_type.from_media_type 'image/png'
utils.assert_eq(png[1], {
  name = 'Portable Network Graphics',
  media_types = { 'image/png' },
  extensions = { 'png' },
})
