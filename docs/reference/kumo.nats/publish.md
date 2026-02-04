### client:publish({PARAMS})
{{since('dev')}}

Sends a message. `PARAMS` is an object style table with the following keys:

* `subject`   - required string; the name of the subject to which to send the message
* `payload`   - required string; the message to send
* `headers`   - optional NATS headers
* `await_ack` - optional wait for server acknowledgement (default: true)

In case `await_ack` is set to `true`, `publish` returns an acknowledgment with the following values:

* `stream`: name of stream the message was published to
* `sequence`: sequence number the message was published in
* `domain`: domain the message was published to
* `deplicate`: true if the published message was determined to be a duplicate, false otherwise
* `value`: used only when published against stream with counters enabled

```lua
nats:publish {
  subject = 'subject',
  payload = 'payload',
  headers = {
      ['Nats-Msg-Id'] = 'unique-id',
  },
  await_ack = true,
}
```