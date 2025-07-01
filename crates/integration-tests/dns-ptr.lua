local kumo = require 'kumo'

local function contains(list, t)
    for key, value in pairs(list) do
        if value == t then
            return true
        end
    end
    return false
end

-- check if we're able to resolve back to localhost
local ok, a = pcall(kumo.dns.lookup_ptr, '127.0.0.1')
assert(ok, 'expected localhost for 127.0.0.1 ptr')
assert(contains(a, 'localhost.'), 'expected localhost.')

-- see if we're able to do resolve for ipv6
local ok, a = pcall(kumo.dns.lookup_ptr, '::1')
assert(ok, 'expected localhost for ::1 ptr')
assert(contains(a, 'localhost.'), 'expected localhost.')

local ok, a = pcall(kumo.dns.lookup_ptr, '::2')
assert(not ok, '::2 should not have ptr')
assert(string.find(tostring(a), 'no records found for Query') ~= nil, 'expected "no records found for Query"')
