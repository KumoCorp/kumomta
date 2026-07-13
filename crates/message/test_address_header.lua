local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'

local function new_msg(content)
  return kumo.make_message('sender@example.com', 'recip@example.com', content)
end

-- HeaderAddressList: single simple address without display name
local msg =
  new_msg 'From: user@example.com\r\nTo: someone@example.com\r\n\r\nBody'
local from = msg:from_header()
utils.assert_eq(from.user, 'user')
utils.assert_eq(from.domain, 'example.com')
utils.assert_eq(from.email, 'user@example.com')
utils.assert_eq(from.name, nil)

-- HeaderAddressList: single address with display name
local msg2 =
  new_msg 'From: "John Smith" <john@example.com>\r\nTo: someone@example.com\r\n\r\nBody'
local from2 = msg2:from_header()
utils.assert_eq(from2.user, 'john')
utils.assert_eq(from2.domain, 'example.com')
utils.assert_eq(from2.email, 'john@example.com')
utils.assert_eq(from2.name, 'John Smith')

-- HeaderAddressList: tostring produces JSON
local json_str = tostring(from2)
utils.assert_eq(
  json_str,
  '[{"Address":{"name":"John Smith","address":"john@example.com"}}]'
)

-- HeaderAddressList: error cases on multi-address header
local msg3 =
  new_msg 'From: a@example.com\r\nTo: alice@example.com, bob@example.com\r\n\r\nBody'
local to_multi = msg3:to_header()

local ok, err = pcall(function()
  local _ = to_multi.user
end)
utils.assert_eq(ok, false, 'multi-address .user should error')

local ok2, _ = pcall(function()
  local _ = to_multi.domain
end)
utils.assert_eq(ok2, false, 'multi-address .domain should error')

local ok3, _ = pcall(function()
  local _ = to_multi.email
end)
utils.assert_eq(ok3, false, 'multi-address .email should error')

local ok4, _ = pcall(function()
  local _ = to_multi.name
end)
utils.assert_eq(ok4, false, 'multi-address .name should error')

-- Missing header returns nil
utils.assert_eq(msg:get_address_header 'X-Nonexistent', nil)

-- HeaderAddressList.list: multi-address
local list = to_multi.list
utils.assert_eq(#list, 2)

-- HeaderAddress fields on list entries
utils.assert_eq(list[1].user, 'alice')
utils.assert_eq(list[1].domain, 'example.com')
utils.assert_eq(list[1].email, 'alice@example.com')
utils.assert_eq(list[1].name, nil)

utils.assert_eq(list[2].user, 'bob')
utils.assert_eq(list[2].domain, 'example.com')
utils.assert_eq(list[2].email, 'bob@example.com')
utils.assert_eq(list[2].name, nil)

-- HeaderAddress with display name via list
local msg4 =
  new_msg 'From: a@a.com\r\nTo: "Alice" <alice@example.com>, bob@example.com\r\n\r\nBody'
local to_named = msg4:to_header()
local named_list = to_named.list
utils.assert_eq(named_list[1].name, 'Alice')
utils.assert_eq(named_list[2].name, nil)

-- HeaderAddress tostring produces JSON
utils.assert_eq(
  tostring(named_list[1]),
  '{"name":"Alice","address":"alice@example.com"}'
)
utils.assert_eq(
  tostring(named_list[2]),
  '{"name":null,"address":"bob@example.com"}'
)

-- from_header() is equivalent to get_address_header("From")
local via_from = msg2:from_header()
local via_get = msg2:get_address_header 'From'
utils.assert_eq(tostring(via_from), tostring(via_get))

-- to_header() is equivalent to get_address_header("To")
local via_to = msg3:to_header()
local via_get_to = msg3:get_address_header 'To'
utils.assert_eq(tostring(via_to), tostring(via_get_to))

-- Single-address list field returns one entry
local single_list = from.list
utils.assert_eq(#single_list, 1)
utils.assert_eq(single_list[1].email, 'user@example.com')

-- Edge case: quoted local part with @ in it
local msg5 =
  new_msg 'From: "info@"@example.com\r\nTo: someone@example.com\r\n\r\nBody'
local from5 = msg5:from_header()
utils.assert_eq(from5.domain, 'example.com')
-- The list entry should also crack correctly
local list5 = from5.list
utils.assert_eq(list5[1].domain, 'example.com')

-- Edge case: group address
local msg6 =
  new_msg 'From: a@a.com\r\nTo: mygroup: alice@example.com, bob@example.com;\r\n\r\nBody'
local to_group = msg6:to_header()

-- .list flattens groups, so we get both members
local group_list = to_group.list
utils.assert_eq(#group_list, 2)
utils.assert_eq(group_list[1].email, 'alice@example.com')
utils.assert_eq(group_list[2].email, 'bob@example.com')

-- single-address accessors should error on a group
local ok5, _ = pcall(function()
  local _ = to_group.user
end)
utils.assert_eq(ok5, false, 'group address .user should error')

-- Edge case: mix of group and regular address
local msg7 =
  new_msg 'From: a@a.com\r\nTo: mygroup: alice@example.com, bob@example.com;, carol@example.com\r\n\r\nBody'
local to_mixed = msg7:to_header()
local mixed_list = to_mixed.list
utils.assert_eq(#mixed_list, 3)
utils.assert_eq(mixed_list[1].email, 'alice@example.com')
utils.assert_eq(mixed_list[2].email, 'bob@example.com')
utils.assert_eq(mixed_list[3].email, 'carol@example.com')
