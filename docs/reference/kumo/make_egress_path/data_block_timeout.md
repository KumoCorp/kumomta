# data_block_timeout

{{since('dev')}}

How long to wait for the destination to accept each block of the message data
that is sent after the `DATA` command. The default is `60s`.

The message is written to the connection in blocks of up to 64KiB, and this
timeout applies to each block separately. It limits how long a transfer may go
without making progress; it does *not* limit how long the transfer as a whole
may take. A slow but steady receiver can therefore accept a large message, no
matter how long that takes, provided that it keeps accepting data. A receiver
that stops accepting data is detected after this timeout elapses and the
connection is closed, and the message is subject to the usual retry logic.

This is the behavior that [RFC 5321 section
4.5.3.2.4](https://www.rfc-editor.org/rfc/rfc5321#section-4.5.3.2.4)
describes for the data block timeout, which it recommends should be at least 3
minutes. If you deliver to destinations that are slow to accept data, you may
wish to raise this value.

```lua
kumo.on('get_egress_path_config', function(domain, source_name, site_name)
  return kumo.make_egress_path {
    data_block_timeout = '3 minutes',
  }
end)
```

Other timeouts apply to the other parts of sending a message:

* [data_timeout](data_timeout.md) is how long to wait for the response to the
  `DATA` command itself.
* [data_dot_timeout](data_dot_timeout.md) is how long to wait for the response
  after the message data has been sent, which is when the destination
  processes the message.

While message data is being written, the connection is considered to be making
forward progress for the purposes of
[dispatcher_progress_watchdog_timeout](dispatcher_progress_watchdog_timeout.md),
so a long transfer to a slow receiver is not aborted by the watchdog.

This option is not included in the default value of
[system_shutdown_timeout](system_shutdown_timeout.md), because it bounds a
single block rather than the whole transfer.
