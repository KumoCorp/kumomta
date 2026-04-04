---
tags:
 - utility
 - logging
 - jsonl
---

# `kumo.jsonl.new_tailer`

```lua
local tailer = kumo.jsonl.new_tailer {
  directory = '/path/to/logs',
  -- optional fields:
  pattern = '*.zst',
  max_batch_size = 100,
  max_batch_latency = '1s',
  checkpoint_name = 'my-consumer',
  poll_watcher = '500ms',
  tail = false,
}
-- optional filter as second argument:
local tailer2 = kumo.jsonl.new_tailer(
  { directory = '/var/log/kumomta' },
  function(record)
    return record.type == 'Delivery'
  end
)
```

{{since('dev')}}

Creates a single-consumer log tailer that reads zstd-compressed JSONL segment
files from `directory` and yields them as batches of parsed JSON values.

Progress can be checkpointed so that the tailer resumes from where it left off
after a restart.  An optional filter function can be supplied to discard
unwanted records before they are added to a batch.

## Configuration Parameters

### directory

*String.* Required. The directory containing the segment files to read.

### pattern

*String (glob).* Optional. A glob pattern to match segment files within
`directory`. Defaults to `"*"`.

### max_batch_size

*Integer.* Optional. Maximum number of records per batch. Defaults to `100`.

### max_batch_latency

*Duration string* (e.g., `"500ms"`, `"1s"`). Optional. Maximum time to wait
for a partial batch to fill before yielding it. Defaults to `"1s"`.

### checkpoint_name

*String.* Optional. When set, enables checkpoint persistence.  The checkpoint
is stored as a hidden file named `.<checkpoint_name>` inside `directory`.  On
the next run, the tailer resumes from the checkpointed position rather than
re-reading from the beginning.

### poll_watcher

*Duration string* (e.g., `"500ms"`). Optional. When set, uses a polling-based
filesystem watcher with the given interval instead of the platform's native
filesystem notification mechanism.  Useful on network filesystems or container
environments where native watchers are unreliable.

### tail

*Boolean.* Optional. When `true`, ignores any existing checkpoint and starts
reading from the most recent segment only, skipping all older segments.
Defaults to `false`.

## Filter function

An optional second argument can be a function that receives each parsed record
as a lua value and returns `true` to include it in the batch or `false` to
discard it.  If the function raises an error, the error is propagated to the
caller.

## Methods

### tailer:batches

```lua
for batch in tailer:batches() do
  -- batch is a LogBatch object; see LogBatch docs
end
```

Returns an iterator function that yields one [`LogBatch`](LogBatch.md) per
call, or `nil` when the stream is exhausted.  Each call polls the underlying
stream for more data.

### tailer:close

```lua
tailer:close()
```

Signals the tailer to stop.  Any pending or subsequent poll returns `nil`.

Note: `:close()` does **not** write a checkpoint.  Call
[`batch:commit()`](LogBatch.md#batchcommit) on each batch after processing it
to advance the checkpoint.

## Example

```lua
local kumo = require 'kumo'

local tailer = kumo.jsonl.new_tailer {
  directory = '/var/log/kumomta',
  pattern = '*.zst',
  max_batch_size = 500,
  max_batch_latency = '250ms',
  checkpoint_name = 'delivery-processor',
}

for batch in tailer:batches() do
  for record in batch:iter_records() do
    if record.type == 'Delivery' then
      print('delivered: ' .. record.id)
    end
  end
  batch:commit()
end

tailer:close()
```

## Batched Webhook Example

The following example shows how to use `new_tailer` to read log records from
disk and post them in batches to an HTTP endpoint, equivalent to the
[batched webhook](../../userguide/operation/webhooks.md#batched-hooks)
approach but driven from the log files rather than an in-process hook.

Each batch is encoded as a JSON array and posted as the request body, for
example: `[{"type": "Delivery", ...}, {"type": "Reception", ...}]`.

Save the script below as e.g. `/path/to/webhook.lua` and run it as a
standalone script:

```console
$ /opt/kumomta/sbin/kumod --script --policy /path/to/webhook.lua
```

```lua
local kumo = require 'kumo'

kumo.on('main', function()
  local client = kumo.http.build_client {}

  local tailer = kumo.jsonl.new_tailer {
    directory = '/var/log/kumomta',
    max_batch_size = 100,
    max_batch_latency = '1s',
    checkpoint_name = 'webhook-poster',
  }

  for batch in tailer:batches() do
    -- Collect the records into a lua table, then JSON-encode
    -- the whole array as the request body.
    local payload = kumo.serde.json_encode(batch:records())

    local response = client
      :post('http://10.0.0.1:4242/log')
      :header('Content-Type', 'application/json')
      :body(payload)
      :send()

    if response:status_is_success() then
      -- Only advance the checkpoint once we know the
      -- remote endpoint accepted the batch.
      batch:commit()
    else
      -- We did not commit, so the next run will retry this batch.
      error(
        string.format(
          'webhook post failed: %d %s: %s',
          response:status_code(),
          response:status_reason(),
          response:text()
        )
      )
    end
  end

  tailer:close()
  client:close()
end)
```
