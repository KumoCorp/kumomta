# builder

```lua
kumo.mimepart.builder()
```

{{since('2025.10.06-5ec871ab')}}

Returns a message builder object that can be used to build an email from an
optional text, optional html and optional set of attachments. The builder takes
care of phrasing the resulting mime structure in the most commonly accepted
arrangement to reflect your combination of text, html and attachments.

For example, if you specify both an html and a text part, they will be wrapped
up in a `multipart/alternative` container part.  If you add attachments, then a
`multipart/mixed` container will be created.

## Available Methods

  * `builder:text_plain(TEXT)` - call this to set the plain text part of the message. The part is constructed as though you called [kumo.mimepart.new_text_plain](new_text_plain.md) with the same parameters.
  * `builder:text_html(HTML)` - call this to set the HTML part of the message. The part is constructed as though you called [kumo.mimepart.new_html](new_html.md) with the same parameters.
  * `builder:attach(CONTENT_TYPE, DATA, ATTACHMENT_OPTIONS)` - call this to add an attachment. The part is constructed as though you called [kumo.mimepart.new_binary](new_binary.md) with the same parameters.
  * `builder:attach_part(PART)` - call this to add an attachment part that you have constructed separately
  * `builder:set_stable_content(true)` - provided for testing purposes, when called and set to `true`, the generated boundaries and other automatically added headers will used fixed strings to aid in making test assertions.
  * `builder:build()` - finalize the message, consuming its internal state, and return the newly constructed [MimePart](../mimepart/index.md).
  * `builder:text_amp_html(AMP_HTML)` - call this to set the [AMP
    HTML](https://amp.dev/documentation/guides-and-tutorials/email/learn/email-spec/amp-email-structure)
    part of the message. The part is constructed as though you created a new
    text part with the content type `text/x-amp-html`. {{since('dev', inline=True)}}.

## Example

```lua
-- Build up an email with a couple of attachments/parts
local builder = kumo.mimepart.builder()
builder:text_plain 'Hello, plain'
builder:text_html '<b>Hello, html</b>'
builder:attach(
  'text/plain',
  'I am a plain text file with no options specified'
)
builder:attach('application/octet-stream', '\xaa\xbb', {
  file_name = 'binary.dat',
})
local root = builder:build()
print(root)
```

Will produce a message looking something like this; the `boundary` strings will vary:

```
Content-Type: multipart/mixed;
   boundary="mm-boundary"
Mime-Version: 1.0
Date: Tue, 1 Jul 2003 10:52:37 +0200

--mm-boundary
Content-Type: multipart/alternative;
   boundary="ma-boundary"

--ma-boundary
Content-Type: text/plain;
   charset="us-ascii"

Hello, plain
--ma-boundary
Content-Type: text/html;
   charset="us-ascii"

<b>Hello, html</b>
--ma-boundary--
--mm-boundary
Content-Type: text/plain
Content-Transfer-Encoding: base64

SSBhbSBhIHBsYWluIHRleHQgZmlsZSB3aXRoIG5vIG9wdGlvbnMgc3BlY2lmaWVk
--mm-boundary
Content-Type: application/octet-stream
Content-Transfer-Encoding: base64
Content-Disposition: attachment;
   filename="binary.dat"

qrs=
--mm-boundary--

```
