# system_shutdown_timeout

{{since('2025.05.06-b29689af')}}

How long to wait for an in-flight delivery attempt to gracefully complete when
the system is being shutdown. Once this timeout is reached, any open sessions
will be aborted.

The default value is computed by summing up the per-message-delivery timeout
values:

* [mail_from_timeout](mail_from_timeout.md) +
* [rcpt_to_timeout](rcpt_to_timeout.md) +
* [data_timeout](data_timeout.md) +
* [data_dot_timeout](data_dot_timeout.md)

!!! caution
    If you make this value too short you increase the risk duplicate delivery
    when the sessions are terminated.



