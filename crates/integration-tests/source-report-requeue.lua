local kumo = require 'kumo'
dofile 'source.lua'

-- Report the callback's response and remaining recipients in the policy bounce.
kumo.on('requeue_message', function(_msg, response)
  kumo.reject(550, 'requeue_message saw: ' .. response)
end)
