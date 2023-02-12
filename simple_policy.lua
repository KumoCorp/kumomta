-- Called to validate the helo and/or ehlo domain
on("smtp_server_ehlo", function(domain)
  print("ehlo domain is", domain)
end)

-- Called to validate the sender
on("smtp_server_mail_from", function(sender)
  print("sender", tostring(sender))
end)

-- Called to validate a recipient
on("smtp_server_mail_rcpt_to", function(rcpt)
  print("rcpt", tostring(rcpt))
end)

-- Called once the body has been received
on("smtp_server_message_received", function(state)
  print("sender", tostring(state.sender))

  -- set/get metadata fields
  state:meta_set("foo", "bar")
  print("meta foo is", state:meta_get("foo"))
end)
