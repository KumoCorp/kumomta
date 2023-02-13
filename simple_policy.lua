local kumo = require 'kumo'

-- Called on startup to initialize the system
kumo.on("init", function()
  kumo.start_esmtp_listener{
    listen="127.0.0.1:2025"
  }
end)

-- Called to validate the helo and/or ehlo domain
kumo.on("smtp_server_ehlo", function(domain)
  print("ehlo domain is", domain)
end)

-- Called to validate the sender
kumo.on("smtp_server_mail_from", function(sender)
  print("sender", tostring(sender))
end)

-- Called to validate a recipient
kumo.on("smtp_server_mail_rcpt_to", function(rcpt)
  print("rcpt", tostring(rcpt))
end)

-- Called once the body has been received
kumo.on("smtp_server_message_received", function(msg)
  print("id", msg:id(), "sender", tostring(msg:sender()))

  -- set/get metadata fields
  msg:set_meta("foo", "bar")
  print("meta foo is", msg:get_meta("foo"))
end)
