# `message:shrink_data()`

{{since('dev')}}

This method will ensure that the message contents are journalled to the spool,
and then release any in-memory body data, keeping the metadata in-memory.

See also:
* [msg:shrink()](shrink.md)

