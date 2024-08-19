# invalid_line_endings

{{since('2023.11.28-b5252a41')}}

Specifies the behavior when the received DATA contains invalid line
endings.  The SMTP protocol requires that each line of the DATA be
separated by canonical CRLF sequences. Immediately after receiving the DATA
payload, but before any other policy events are triggered, if the received
DATA is non-conforming the value of this parameter is checked to determine
what to do. It has three possible values:

* `"Deny"` - this is the default. The incoming message will be
    rejected.
* `"Allow"` - The incoming message will be accepted. Depending
    on the configured policy, some later policy actions may fail
    to parse the message, and DKIM signatures may be created that
    are not possible to validate correctly.  There is no guarantee
    that any resulting message will be routable to its intended
    destination.
* `"Fix"` - the line endings will be normalized to CRLF and the
    message will be accepted.  It's possible for this to invalidate
    any signatures that may have already been present in the message.


