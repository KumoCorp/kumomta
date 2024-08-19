# data_buffer_size

Specified the size of the buffer used to read chunks of the message payload
during the `DATA` phase of the SMTP transaction.  Making this larger will
improve the throughput in terms of bytes-per-syscall at the expense of
using more RAM.

The default size is 128KB (`128 * 1024`).  If your average message size is
significantly larger than the default, then you may wish to increase this
value.


