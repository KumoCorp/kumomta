# `message:parse_rfc3464()`

Parses the message data as an RFC 3464 delivery status report.

If the message is not an RFC 3464 report, returns `nil`.
If the message is malformed, raises a lua error.

Otherwise, returns a lua table that looks like:

```lua
report = {
  per_message = {
    reporting_mta = {
      mta_type = 'dns',
      name = 'cs.utk.edu',
    },
  },
  -- This is an array style table, with one entry per
  -- recipient in the report
  per_recipient = {
    {
      original_recipient = {
        recipient_type = 'rfc822',
        recipient = 'louisl@larry.slip.umd.edu',
      },
      final_recipient = {
        recipient_type = 'rfc822',
        recipient = 'louisl@larry.slip.umd.edu',
      },
      action = 'failed',
      status = {
        class = 4,
        subject = 0,
        detail = 0,
      },
      diagnostic_code = {
        diagnostic_type = 'smtp',
        diagnostic = '426 connection timed out',
      },
      last_attempt_date = '1994-07-07T21:15:49Z',
    },
  },
}
```
