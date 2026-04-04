---
tags:
 - utility
 - logging
 - jsonl
---

# `kumo.jsonl.new_multi_tailer`

```lua
local tailer = kumo.jsonl.new_multi_tailer {
  directory = '/path/to/logs',
  consumers = {
    {
      name = 'deliveries',
      checkpoint_name = 'cp-deliveries',
      max_batch_size = 100,
      max_batch_latency = '1s',
      filter = function(record)
        return record.type == 'Delivery'
      end,
    },
    {
      name = 'bounces',
      checkpoint_name = 'cp-bounces',
      max_batch_latency = '5s',
      filter = function(record)
        return record.type == 'Bounce'
      end,
    },
  },
}
```

{{since('dev')}}

Creates a multi-consumer log tailer that reads each segment file exactly once
and distributes records to multiple independent consumers.  Each consumer has
its own filter, batch parameters, and optional checkpoint.

This is more efficient than running multiple single-consumer tailers over the
same directory, because decompression and JSON parsing happen once per record
regardless of how many consumers are configured.

## Top-level Configuration Parameters

### directory

*String.* Required. The directory containing the segment files to read.

### pattern

*String (glob).* Optional. A glob pattern to match segment files within
`directory`. Defaults to `"*"`.

### poll_watcher

*Duration string* (e.g., `"500ms"`). Optional. When set, uses a polling-based
filesystem watcher with the given interval instead of the platform's native
filesystem notification mechanism.

### tail

*Boolean.* Optional. When `true`, ignores any existing checkpoints and starts
reading from the most recent segment only. Defaults to `false`.

### consumers

*Array of consumer config tables.* Required. Each entry defines one consumer.
See [Consumer Configuration](#consumer-configuration) below.

## Consumer Configuration

Each entry in the `consumers` array is a table with the following fields.

### name

*String.* Required. A unique name for this consumer.  Returned by
[`batch:consumer_name()`](LogBatch.md#batchconsumer_name) so that the caller
can route batches correctly.

### max_batch_size

*Integer.* Optional. Maximum number of records per batch for this consumer.
Defaults to `100`.

### max_batch_latency

*Duration string.* Optional. Maximum time to wait for a partial batch to fill
before yielding it.  Defaults to `"1s"`.

Consumers with different latencies will have their batches yielded
independently: a consumer with a short latency does not delay a consumer with a
longer one.

### checkpoint_name

*String.* Optional. When set, enables checkpoint persistence for this consumer
stored as a hidden file `.<checkpoint_name>` inside `directory`.  Each consumer
has its own independent checkpoint.

### filter

*Function.* Optional. A function that receives each parsed record as a lua
value and returns `true` to include it in this consumer's batch or `false` to
discard it.  If the function raises an error, the error is propagated to the
caller.

## Methods

### tailer:batches

```lua
for batches in tailer:batches() do
  for _, batch in ipairs(batches) do
    print(batch:consumer_name())
  end
end
```

Returns an iterator that yields a lua table (sequence) of
[`LogBatch`](LogBatch.md) objects on each call — one entry per consumer whose
batch is ready — or `nil` when the stream is exhausted.

A batch is ready when it is full (`max_batch_size` reached) or its
`max_batch_latency` has expired.  Consumers are yielded independently: a slow
consumer does not prevent a fast consumer's batch from being delivered.

### tailer:close

```lua
tailer:close()
```

Signals the tailer to stop.  Any pending or subsequent poll returns `nil`.

Note: `:close()` does **not** write a checkpoint.  Call
[`batch:commit()`](LogBatch.md#batchcommit) on each batch after processing it
to advance that consumer's checkpoint.

## Example

```lua
local kumo = require 'kumo'

local tailer = kumo.jsonl.new_multi_tailer {
  directory = '/var/log/kumomta',
  pattern = '*.zst',
  consumers = {
    {
      name = 'deliveries',
      checkpoint_name = 'cp-del',
      max_batch_size = 200,
      max_batch_latency = '500ms',
      filter = function(record)
        return record.type == 'Delivery'
      end,
    },
    {
      name = 'bounces',
      checkpoint_name = 'cp-bounce',
      max_batch_latency = '5s',
      filter = function(record)
        return record.type == 'Bounce' or record.type == 'TransientFailure'
      end,
    },
  },
}

for batches in tailer:batches() do
  for _, batch in ipairs(batches) do
    local name = batch:consumer_name()
    for record in batch:iter_records() do
      print(name .. ': ' .. record.id)
    end
    batch:commit()
  end
end

tailer:close()
```
