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
  local max_retries = 5
  local retry_delay_seconds = 2

  -- Perform the HTTP POST and return the response.
  local function post_batch(payload)
    return client
      :post('http://10.0.0.1:4242/log')
      :header('Content-Type', 'application/json')
      :body(payload)
      :send()
  end

  -- Encode the records as a JSON array, then post with retries.
  -- Returns only if the post succeeded; raises an error otherwise,
  -- leaving the batch uncommitted so the next run will retry it.
  local function process_batch(records)
    local payload = kumo.serde.json_encode(records)

    for attempt = 1, max_retries do
      local response = post_batch(payload)

      if response:status_is_success() then
        return
      end

      local status = response:status_code()
      local disposition = string.format(
        '%d %s: %s',
        status,
        response:status_reason(),
        response:text()
      )

      -- 429 Too Many Requests and 5xx server errors are
      -- transient and worth retrying.  4xx client errors
      -- (other than 429) indicate a permanent problem with
      -- the request itself and are not worth retrying.
      local retryable = status == 429 or status >= 500
      if not retryable or attempt == max_retries then
        -- We did not commit, so the next run will retry this batch.
        -- Sleep briefly before failing out so that a supervising
        -- process does not restart us too quickly.
        kumo.log_error(
          string.format(
            'webhook post failed (attempt %d/%d), giving up: %s',
            attempt,
            max_retries,
            disposition
          )
        )
        kumo.time.sleep(5)
        error('webhook post failed: ' .. disposition)
      end

      -- Transient failure: wait before retrying.
      kumo.log_error(
        string.format(
          'webhook post failed (attempt %d/%d), retrying in %ds: %s',
          attempt,
          max_retries,
          retry_delay_seconds,
          disposition
        )
      )
      kumo.time.sleep(retry_delay_seconds)
    end
  end

  local tailer = kumo.jsonl.new_tailer(
    {
      directory = '/var/log/kumomta',
      max_batch_size = 100,
      max_batch_latency = '1s',
      checkpoint_name = 'webhook-poster',
    },
    -- Only forward records for the 'customer-a' egress pool.
    function(record)
      return record.egress_pool == 'customer-a'
    end
  )

  for batch in tailer:batches() do
    process_batch(batch:records())
    batch:commit()
  end

  tailer:close()
  client:close()
end)
```

## Per-Customer Webhook Example with `main` Parameters

The `main` event receives any extra command-line arguments passed after `--`
as parameters to the handler function.  This makes it straightforward to run
one instance of the same script per customer, each operating independently
with its own checkpoint and filter:

```console
$ /opt/kumomta/sbin/kumod --script --policy /path/to/webhook.lua -- customer-a
$ /opt/kumomta/sbin/kumod --script --policy /path/to/webhook.lua -- customer-b
$ /opt/kumomta/sbin/kumod --script --policy /path/to/webhook.lua -- customer-c
```

Each process reads only the records for its own pool, maintains its own
checkpoint, and retries failures independently without affecting any other
customer's delivery.  This is the robust alternative to the multi-consumer
tailer approach described in
[`kumo.jsonl.new_multi_tailer`](new_multi_tailer.md#per-customer-batched-webhook-example).

```lua
local kumo = require 'kumo'

-- Map each customer (egress pool name) to their webhook endpoint.
local endpoints = {
  ['customer-a'] = 'http://customer-a.example.com/log',
  ['customer-b'] = 'http://customer-b.example.com/log',
  ['customer-c'] = 'http://customer-c.example.com/log',
}

kumo.on('main', function(pool_name)
  local url = assert(
    endpoints[pool_name],
    string.format("unknown pool '%s'", pool_name)
  )

  local client = kumo.http.build_client {}
  local max_retries = 5
  local retry_delay_seconds = 2

  local function post_batch(payload)
    return client
      :post(url)
      :header('Content-Type', 'application/json')
      :body(payload)
      :send()
  end

  -- Returns only if the post succeeded; raises an error otherwise,
  -- leaving the batch uncommitted so the next run will retry it.
  local function process_batch(records)
    local payload = kumo.serde.json_encode(records)

    for attempt = 1, max_retries do
      local response = post_batch(payload)

      if response:status_is_success() then
        return
      end

      local status = response:status_code()
      local disposition = string.format(
        '%d %s: %s',
        status,
        response:status_reason(),
        response:text()
      )

      local retryable = status == 429 or status >= 500
      if not retryable or attempt == max_retries then
        kumo.log_error(
          string.format(
            'webhook post to %s failed (attempt %d/%d), giving up: %s',
            url,
            attempt,
            max_retries,
            disposition
          )
        )
        kumo.time.sleep(5)
        error('webhook post failed: ' .. disposition)
      end

      kumo.log_error(
        string.format(
          'webhook post to %s failed (attempt %d/%d), retrying in %ds: %s',
          url,
          attempt,
          max_retries,
          retry_delay_seconds,
          disposition
        )
      )
      kumo.time.sleep(retry_delay_seconds)
    end
  end

  local tailer = kumo.jsonl.new_tailer(
    {
      directory = '/var/log/kumomta',
      max_batch_size = 100,
      max_batch_latency = '1s',
      -- Each customer has its own independent checkpoint.
      checkpoint_name = 'webhook-' .. pool_name,
    },
    -- Only process records belonging to this customer's egress pool.
    function(record)
      return record.egress_pool == pool_name
    end
  )

  for batch in tailer:batches() do
    process_batch(batch:records())
    batch:commit()
  end

  tailer:close()
  client:close()
end)
```
