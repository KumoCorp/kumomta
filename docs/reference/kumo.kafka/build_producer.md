# `kumo.kafka.build_producer(URI)`

Constructs an AMQP client object.

`URI` is the URI that references the AMQP server to which you want to connect.

```lua
local producer = kumo.kafka.build_producer {
  ['bootstrap.servers'] = 'localhost:9092',
}
```

## Client Methods

The returned client object has the following methods:

### client:send({PARAMS})

Sends a message. `PARAMS` is an object style table with the
following keys:

* `topic` - required string; the name of the queue to which to send the message
* `payload` - required string; the message to send
* `timeout` - how long to wait for a response.

The result from send is a tuple local partition, offset = producer:send {...}.

```lua
local producer = kumo.kafka.build_producer {
  ['bootstrap.servers'] = 'localhost:9092',
}

producer:send {
  topic = 'my.topic',
  payload = message:get_data(),
  -- how long to keep trying to submit to kafka
  -- before a lua error will be raised.
  -- This is the default.
  timeout = '1 minute',
}
```

### client:close()

{{since('2024.09.02-c5476b89')}}

Explicitly close the client object and associated connection.
