# try_next_host_on_transport_error

{{since('dev')}}

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

In earlier versions of KumoMTA this option did not exist and the system behaves
as if it was set to `false`.
