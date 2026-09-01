# VERP (Variable Envelope Return Path)

VERP is the technique of encoding each recipient's address into a unique envelope sender, so that any bounce identifies exactly which address failed without parsing the bounce text. A message to user@example.com might carry a return path of bounces+user=example.com@sender.example. VERP makes bounce attribution mechanical and reliable, at the cost of a very large number of distinct return-path addresses to accept.
