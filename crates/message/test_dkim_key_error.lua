local kumo = require 'kumo'

kumo.on('test-signing-key', function(token)
  assert(token == 'private-argument')
  return 'not a private key'
end)

kumo.on('main', function()
  local ok, err = pcall(kumo.dkim.rsa_sha256_signer, {
    domain = 'example.com',
    selector = 's1',
    headers = { 'From' },
    key = {
      event_name = 'test-signing-key',
      event_args = { 'private-argument' },
    },
  })
  assert(not ok)
  local detail = tostring(err)
  assert(detail:find('failed to load private key', 1, true), detail)
  assert(detail:find('test-signing-key', 1, true), detail)
  assert(not detail:find('private-argument', 1, true), detail)
end)
