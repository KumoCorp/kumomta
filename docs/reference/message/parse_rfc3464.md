# parse_rfc3464

```lua
message:parse_rfc3464()
```

Parses the message data as an RFC 3464 delivery status report.

If the message is not an RFC 3464 report, returns `nil`.
If the message is malformed, raises a lua error.

For `Original-Recipient` and `Final-Recipient`, the `rfc822` address type is
recognized case-insensitively, as required by RFC 3464 section 2.1.2.
Surrounding whitespace is ignored when recognizing the type. These addresses
are parsed as SMTP envelope addresses, so `<User@example.com>` is returned as
`User@example.com`. Local-part case and the trimmed spelling of `recipient_type`
are preserved; domains use the SMTP parser's canonical representation. Other
address types are not normalized as SMTP addresses.

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
