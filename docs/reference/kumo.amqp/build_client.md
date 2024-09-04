# `kumo.amqp.build_client(URI)`

Constructs an AMQP client object, using the underlying
[lapin](https://docs.rs/lapin/) client implementation.

`URI` is the URI that references the AMQP server to which you want to connect.

```lua
local client = kumo.amqp.build_client 'amqp://localhost'
local confirm = client:publish {
  routing_key = 'hello',
  payload = 'w00t!',
}
local result = confirm:wait()
assert(result.status == 'NotRequested')
```

## Client Methods

The returned client object has the following methods:

### client:publish({PARAMS})

Publishes a message. `PARAMS` is an object style table with the
following keys:

* `routing_key` - required string; the name of the queue to which to send the message
* `payload` - required string; the message to send
* `exchange` - optional string; the exchange through which to send the message.
  If unspecified, the empty string is used, which corresponds to a default
  exchange.

Returns a confirmation object that can be used to await the final disposition
of the send.  That confirmation object has a single `wait` method which returns
a confirmation object with the following fields:

* `status` - one of `"NotRequested"`, `"Ack"`, or `"Nack"` depending on the
  disposition of the message delivery attempt.
* `reply_code` - may be nil, but is otherwise a status code from the ack
  returned from the queue machinery.
* `reply_text` - may be nil, but is otherwise status text from the ack
  returned from the queue machinery.

```lua
local client = kumo.amqp.build_client 'amqp://localhost'
local confirm = client:publish {
  routing_key = 'hello',
  payload = 'w00t!',
}
local result = confirm:wait()
assert(result.status == 'NotRequested')
```

### client:close()

{{since('2024.09.02-c5476b89')}}

Explicitly and cleanly closes the connection to the AMQP server.
Calling it multiple times will yield an error.

