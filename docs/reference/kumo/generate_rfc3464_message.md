# kumo.generate_rfc3464_message

```lua
local bounce_msg =
  kumo.generate_rfc3464_message(PARAMS, OPT_ORIG_MSG, LOG_RECORD)
```

{{since('dev')}}

Generates an RFC 3464 delivery status [Message](../message/index.md) from a log
record.  This function is intended to be used from inside a `log_disposition`
event handler to facilitate the generation of bounce messages.

For sender-oriented use cases we generally recommend against generating bounce
messages as they place additional load on the system and introduce some degree
of [Backscatter](https://en.wikipedia.org/wiki/Backscatter_(email)) risk.

When KumoMTA is deployed as a more general purpose relay it becomes more
desirable to generate delivery status reports to the originating user, which is
where this function comes in.

`PARAMS` is an object style table with the following fields:

  * `include_original_message` is a string that can be one of `FullContent`,
    `HeadersOnly` or `No`.  This controls whether the entire message, just its
    headers, or none of the message that triggered the report should be
    included within the report.  When set to anything other than `No`, the
    `OPT_ORIG_MSG` parameter must also be passed to `kumo.generate_rfc3464_message`.
  * `enable_expiration` is an optional boolean value that defaults to `false`
    if not specified. When `true`, a report message will be generated for
    messages that have exceeded their configured time in the queue without
    delivery.
  * `enable_bounce` is an optional boolean value that defaults to `false`
    if not specified. When `true`, a report message will be generated for
    messages that experience a permanent failure response when talking
    to the next hop MTA.
  * `reporting_mta` is a required lua table with the fields `mta_type` and
    `name` that will be included in the `Reporting-MTA` header of the generated
    report.  `mta_type` will typically be `dns` and `name` will typically be
    the corresponding DNS name that identifies the current host generating the
    report.  The domain name will appear in the `From` header of the generated
    report as well.
  * `stable_content` is an optional boolean value that defaults to `false`
    if not specified. When `true`, various header fields will be set to constant
    values that will make the generated message easier to reason about in a
    test harness. You will not typically need to use this parameter.

The `OPT_ORIG_MSG` parameter is an optional [Message](../message/index.md) that
will used to provide the original message content in the report.

`LOG_RECORD` is a [JsonLogRecord](../log_record.md) describing the event that
occurred to the message.

`kumo.generate_rfc3464_message` takes those three parameters and produces an
optional [Message](../message/index.md) from them.

The return value may be `nil`, for example, if the `LOG_RECORD` doesn't match
any of the types that are enabled in the `PARAMS`, or if there is some
information missing or otherwise inapplicable; for example, if `LOG_RECORD` is
a `Bounce` but for a protocol other than `ESMTP` then no report will be
generated.

## Example of generating non-delivery reports

```lua
local log_hooks = require 'policy-extras.log_hooks'

local function ndr_generator(msg, log_record)
  local params = {
    include_original_message = 'FullContent',
    enable_expiration = true,
    enable_bounce = true,
    reporting_mta = {
      mta_type = 'dns',
      name = 'mta1.example.com',
    },
  }
  local bounce_msg = kumo.generate_rfc3464_message(params, msg, log_record)
  if bounce_msg then
    local ok, err = pcall(kumo.inject_message, bounce_msg)
    if not ok then
      kumo.log_error('failed to inject NDR: ', err)
    end
  end
end

log_hooks:new_disposition_hook {
  name = 'ndr_generator',
  hook = ndr_generator,
}
```

If you wish to customize the human readable portion of the message, you might
consider using the mime parsing functions:

```lua
local mime = bounce_msg:parse_mime()
local structure = mime:get_simple_structure()
structure.text_part.body = 'MODIFIED!\r\n' .. structure.text_part.body
bounce_msg:set_data(tostring(mime))
```

## Sample of a generated bounce report

```
Content-Type: multipart/report;
  boundary="report-boundary";
  report-type="delivery-status"
Subject: Returned mail
Mime-Version: 1.0
Message-ID: <UUID@mta1.example.com>
To: sender@sender.example.com
From: Mail Delivery Subsystem <mailer-daemon@mta1.example.com>

--report-boundary
Content-Type: text/plain;
  charset="us-ascii"

The message was received at Tue, 1 Jul 2003 08:52:37 +0000
from sender@sender.example.com and addressed to recip@target.example.com.
While communicating with target.example.com (42.42.42.42):
Response: 550 5.7.1 no thanks

The message will be deleted from the queue.
No further attempts will be made to deliver it.
--report-boundary
Content-Type: message/delivery-status;
  charset="us-ascii"

Reporting-MTA: dns; mta1.example.com
Arrival-Date: Tue, 1 Jul 2003 08:52:37 +0000

Final-Recipient: rfc822;recip@target.example.com
Action: failed
Status: 5.7.1 no thanks
Remote-MTA: dns; target.example.com
Diagnostic-Code: smtp; 550 5.7.1 no thanks
Last-Attempt-Date: Tue, 1 Jul 2003 10:52:37 +0000

--report-boundary
Content-Type: message/rfc822

Content-Type: text/plain;
  charset="us-ascii"
Subject: Hello!

hello there
--report-boundary--

```

## Sample of a generated expiration report

```
Content-Type: multipart/report;
  boundary="report-boundary";
  report-type="delivery-status"
Subject: Returned mail
Mime-Version: 1.0
Message-ID: <UUID@mta1.example.com>
To: sender@sender.example.com
From: Mail Delivery Subsystem <mailer-daemon@mta1.example.com>

--report-boundary
Content-Type: text/plain;
  charset="us-ascii"
Content-Transfer-Encoding: quoted-printable

The message was received at Tue, 1 Jul 2003 08:52:37 +0000
from sender@sender.example.com and addressed to recip@target.example.com.
Status: 551 5.4.7 Next delivery time would be at SOME TIME which exceeds th=
e expiry time EXPIRES configured via set_scheduling
The message will be deleted from the queue.
No further attempts will be made to deliver it.
--report-boundary
Content-Type: message/delivery-status;
  charset="us-ascii"
Content-Transfer-Encoding: quoted-printable

Reporting-MTA: dns; mta1.example.com
Arrival-Date: Tue, 1 Jul 2003 08:52:37 +0000

Final-Recipient: rfc822;recip@target.example.com
Action: failed
Status: 5.4.7 Next delivery time would be at SOME TIME which exceeds the ex=
piry time EXPIRES configured via set_scheduling
Diagnostic-Code: smtp; 551 5.4.7 Next delivery time would be at SOME TIME w=
hich exceeds the expiry time EXPIRES configured via set_scheduling
Last-Attempt-Date: Tue, 1 Jul 2003 10:52:37 +0000

--report-boundary
Content-Type: text/rfc822-headers

Content-Type: text/plain;
  charset="us-ascii"
Subject: Hello!
--report-boundary--
```

