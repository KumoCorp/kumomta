# try_next_host_on_transport_error

{{since('2025.10.06-5ec871ab')}}

An optional boolean value that defaults to `false`.

When set to `true`, if an SMTP message delivery attempt
encounters a timeout, a transport error, or a protocol error that isn't
directly associated with the message (eg: rejection prior to `MAIL FROM`),
then, after logging a `TransientFailure` for the failed attempt, the message
will be immediately eligible for delivery on the next available connection in
the current connection session.

If there are no further available connections, or `try_next_host_on_transport_error` is
set to `false`, then the message will be re-queued to the scheduld queue for
later delivery.

If `MAIL FROM` succeeded, every recipient was refused, and the server still
replied `354` to `DATA`, the client sends only the final dot. A timeout or
disconnect at that dot does not itself trigger a retry on the next host.
The recipient replies determine the outcome: for example, an ordinary `450`
at `RCPT TO` is retried on the scheduled queue even with this option enabled.

In earlier versions of KumoMTA this option did not exist and the system behaves
as if it was set to `false`.
