# trace_headers

Controls the addition of tracing headers to received messages.

KumoMTA can add two different headers to aid in later tracing:

* The standard `"Received"` header which captures SMTP relay hops on their path to the inbox
* A supplemental header which can be used to match feedback reports back to the
  originating mailing

Prior to triggering the
[smtp_server_message_received](../../events/smtp_server_message_received.md)
event the standard `"Received"` header will be added to the
message.  Then, once the event completes and your policy has had the
opportunity to alter the meta data associated with the message, the
supplemental header will be added.

```lua
kumo.start_esmtp_listener {
  -- ..
  trace_headers = {
    -- this is the default: add the Received: header
    received_header = true,

    -- this is the default: add the supplemental header
    supplemental_header = true,

    -- this is the default: the name of the supplemental header
    header_name = 'X-KumoRef',

    -- names of additional meta data fields
    -- to include in the header. TAKE CARE! The header will be
    -- base64 encoded to prevent casual introspection, but the
    -- header is NOT encrypted and the values of the meta data
    -- fields included here should be considered to be public.
    -- The default is not to add any meta data fields, but you
    -- might consider setting something like:
    -- include_meta_names = { 'tenant', 'campaign' },
    include_meta_names = {},
  },
}
```

Here's an example of a supplemental header from a message:

```
X-KumoRef: eyJfQF8iOiJcXF8vIiwicmVjaXBpZW50IjoidGVzdEBleGFtcGxlLmNvbSJ9
```

the decoded payload contains a magic marker key as well as the recipient of the
original message:

```json
{"_@_":"\\_/","recipient":"test@example.com"}
```

Any meta data fields that were listed in `include_meta_names`, if the corresponding
meta data was set in the message, would also be captured in the decoded payload.

KumoMTA will automatically extract this supplemental trace header information
from any `X-` header that is successfully parsed and has the magic marker key
when processing the original message payload of an incoming ARF report.


