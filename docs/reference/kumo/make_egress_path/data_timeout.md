# data_timeout
How long to wait for a response after issuing a DATA command. The default is `30s`.

This applies to the response to the `DATA` command only. The time allowed for
sending the message data itself is controlled by
[data_block_timeout](data_block_timeout.md).
