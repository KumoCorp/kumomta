### client:publish({PARAMS})

Sends a message. `PARAMS` is an object style table with the following keys:

* `subject`   - required string; the name of the subject to which to send the message
* `payload`   - required string; the message to send
* `headers`   - optional NATS headers
* `await_ack` - optional wait for server acknowledgement

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