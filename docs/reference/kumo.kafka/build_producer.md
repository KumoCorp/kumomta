# kumo.kafka.build_producer

```lua
kumo.kafka.build_producer(URI)
```

Constructs a Kafka client object.

`URI` is the URI that references the Kafka cluster to which you want to connect. Bootstrap server addresses should be separated by comma.

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

* `topic` - required string; the name of the topic to which to send the message
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

### client:send_batch({PARAMS})

{{ since('2025.01.23-7273d2bc') }}

Sends a batch of messages. `PARAMS` is a table of object style table with the
following keys:

* `topic` - required string; the name of the topic to which to send the message
* `payload` - required string; the message to send
* `timeout` - how long to wait for a response.

The result from send_batch is a tuple of tables: local failed_items, errors = producer:send_batch(...).

```lua
local producer = kumo.kafka.build_producer {
  ['bootstrap.servers'] = 'localhost:9092',
}

local failed_items, errors = producer:send_batch {
  {
    topic = 'my.topic',
    payload = 'payload 1',
    timeout = '1 minute',
  },
  {
    topic = 'my.other.topic',
    payload = 'payload 2',
    timeout = '1 minute',
  },
}
if #failed_items > 0 then
  -- some items failed
  for i, item_idx in ipairs(failed_items) do
    local error = errors[i]
    print(string.format('item idx %d failed: %s', item_idx, error))
  end
end
```

### client:close()

{{since('2024.09.02-c5476b89')}}

Explicitly close the client object and associated connection.
