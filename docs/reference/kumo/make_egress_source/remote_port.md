# remote_port

Optional integer.

If set, will override the remote SMTP port number. This is useful in scenarios
where your network is set to manage the egress address based on port mapping.

This option takes precedence over
[kumo.make_egress_path().smtp_port](../make_egress_path/smtp_port.md).


