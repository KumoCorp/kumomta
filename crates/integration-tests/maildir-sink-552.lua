local kumo = require 'kumo'
dofile 'maildir-sink.lua'

-- Model a peer that uses the same bare 552 for a recipient refusal and batch overflow.
kumo.on('smtp_server_rewrite_response', function(status)
  if status == 452 then
    return 552, 'recipient refused'
  end
end)
