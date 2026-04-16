# check_fix_conformance

```lua
message:check_fix_conformance(CHECKS, FIXES, OPT_SETTINGS)
```

{{since('2023.11.28-b5252a41')}}

!!!warning
    Fixing messages with this method is inherently imperfect: it is based on a
    deliberately relaxed interpretation of the message content and it is
    possible, or even likely, that non-conforming input is parsed in a way
    that results in omitting certain details from the original input.

    The purpose of this method is as a best-effort convenience for correcting
    minor and obviously recognizable issues that cannot easily be resolved at
    the message generation stage.

    It is recommended that you carefully evaluate the effects of this method
    before deploying it in production.

This method serves two related functions:

* To check RFC conformance issues for which you wish to reject the message.
* To correct RFC conformance issues in case you wish to accept the message.

It accepts two parameters that encode the set of conformance issues that are
applicable. The conformance set is represented as a string listing the issues
separated by a `|` character:

```lua
kumo.on('smtp_server_message_received', function(msg)
  local failed = msg:check_fix_conformance(
    -- check for and reject messages with these issues:
    'MISSING_COLON_VALUE',
    -- fix messages with these issues:
    'LINE_TOO_LONG|NAME_ENDS_WITH_SPACE|NEEDS_TRANSFER_ENCODING|NON_CANONICAL_LINE_ENDINGS|MISSING_DATE_HEADER|MISSING_MESSAGE_ID_HEADER|MISSING_MIME_VERSION'
  )
  if failed then
    kumo.reject(552, string.format('5.6.0 %s', failed))
  end
end)
```

The set of supported conformance issues is:

|Label|Meaning|
|-----|-------|
|MISSING_COLON_VALUE|A header was listed with only its name, and without any value. eg: `"Subject"` instead of `"Subject: the subject"`|
|NON_CANONICAL_LINE_ENDINGS|The message contained line endings that were not in the canonical `CRLF` form required by SMTP|
|NAME_ENDS_WITH_SPACE|A header name ended with space instead of a colon. eg: `"Subject :"` instead of `"Subject: "`. While that is valid for HTTP, it is invalid for email.|
|LINE_TOO_LONG|The line length for the body portion exceeds the MIME message wrapping width that is intended to keep message text wrapping within 80 columns.|
|NEEDS_TRANSFER_ENCODING|The parsed content includes 8-bit content and thus needs to have transfer encoding applied in order to safely transit the 7-bit SMTP network|
|MISSING_DATE_HEADER|The `"Date:"` header is not present|
|MISSING_MESSAGE_ID_HEADER|The `"Message-ID:"` header is not present|
|MISSING_MIME_VERSION|The `"Mime-Version:"` header is either not present or is set to some other value than `"1.0"`|

The way this method works is that it will attempt to parse the data associated
with the message into a MIME tree. The parsing stage will accumulate the set of
conformance issues it uncovers as it parses the tree.

Then the `CHECKS` parameter is decoded into the set of issues for which an error
needs to be generated. If there is an intersection between `CHECKS` and the discovered
conformance issues, then an error message is generated, listing the problematic issues.
Here's an example:

```
Message has conformance issues: LINE_TOO_LONG
```

That message is returned from the method as a string so that you can then choose
to issue an appropriate error response. For example:

```lua
local failed = msg:check_fix_conformance(checks, fixes)
if failed then
  -- Rejects with an error like:
  -- `552 5.6.0 Message has conformance issues: LINE_TOO_LONG`
  kumo.reject(552, string.format('5.6.0 %s', failed))
end
```

If after processing the `CHECKS` parameter an error was not returned, the `FIXES`
parameter is consulted; if there is an intersection between `FIXES` and the discovered
conformance issues, then the message will be "fixed".

The strategy for fixing is simple:

* If any of the problems are not due to missing headers:
    * The message will be rebuilt from the parsed tree. This will cause headers and
      parts to be re-encoded to follow the best practices coded into the built-in
      mime message builder.
* Missing headers will be synthesized and added to the rebuilt message
* The resulting message will then be re-encoded as a byte stream and assigned
  as the data that will be saved when the message is ready to be spooled.

!!!note
    Since fixing issues other than missing headers essentially rewrites the
    message, the chances are very high that any digital signature in the
    original message will be invalidated.

## Fixing 8-bit content

{{since('2025.12.02-67ee9e96')}}

The default behavior when fixing `NEEDS_TRANSFER_ENCODING` is to perform a
lossy conversion to UTF-8, replacing any non-UTF-8 bytes with unicode
replacement characters.

"Fixing" 8-bit content in this context is difficult, because there are many
8-bit encodings and it is not always possible to decide what encoding is
associated with a piece of data--that is why MIME defines explicit transfer
encoding and header encoding indicators in its various RFCs.

KumoMTA includes a character set encoding detector function that can be used to
make a guess at the encoding.  You can enable it by passing in an optional
settings parameter.

Here's an example:

```lua
msg:check_fix_conformance('', 'NEEDS_TRANSFER_ENCODING', {
  -- Enable the encoding detector
  detect_encoding = true,
  -- Constrain the set of allowed encodings to just latin-1.
  -- You can omit the include_encodings option and the full
  -- set of encodings will be considered
  include_encodings = {
    'iso-8859-1',
  },
  -- You can optionally exclude encodings
  exclude_encodings = {},
})
```

Since there is some ambiguity in charset detection, it is recommended that you
employ some heuristics to select the include/exclude list.  For example, if you
know that the sender is from a chinese locale, then you might select `big5` and
not include any latin charsets.

It is not recommended to attempt fixing up the charset of 8-bit content if you
are unsure of the sender.

The set of encodings supported by the detector are:

 * iso-8859-2
 * iso-8859-3
 * iso-8859-4
 * iso-8859-5
 * iso-8859-6
 * iso-8859-7
 * iso-8859-8
 * iso-8859-10
 * iso-8859-13
 * iso-8859-14
 * iso-8859-15
 * iso-8859-16
 * koi8-r
 * koi8-u
 * macintosh
 * windows-874
 * windows-1250
 * windows-1251
 * windows-1252 (which is a superset of iso-8859-1)
 * windows-1253
 * windows-1254
 * windows-1255
 * windows-1256
 * windows-1257
 * windows-1258
 * x-mac-cyrillic
 * gbk
 * gb18030
 * big5
 * euc-jp
 * euc-kr
 * iso-2022-jp
 * shift_jis
 * utf-16be
 * utf-16le
 * utf-8

