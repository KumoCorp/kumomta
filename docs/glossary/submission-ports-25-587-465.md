# Submission ports (25, 587, 465)

Port 25 is for server-to-server SMTP transfer; port 587 is the standard submission port for authenticated clients; port 465 is implicit-TLS submission. Cloud providers commonly block outbound port 25 by default, which is why running an outbound MTA in the cloud starts with getting port-25 egress approved or relaying through hosts that have it.
