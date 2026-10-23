---
tags:
 - utility
 - logging
 - jsonl
---

# LogBatch

{{since('2026.04.09-ea3b2a9b')}}

A `LogBatch` is an object yielded by the `:batches()` method of
[`kumo.jsonl.new_tailer`](new_tailer.md) and
[`kumo.jsonl.new_multi_tailer`](new_multi_tailer.md).
It is not constructed directly.

Each batch contains a set of parsed JSON records read from one or more log
segments, along with metadata about which segment files they came from.

After processing a batch, call `:commit()` to advance the checkpoint so that
the records will not be re-read on the next run.  If `:commit()` is not called
the checkpoint remains at its prior position and the batch will be re-read on
restart.

## Methods

### batch:records

```lua
local recs = batch:records()
for i, record in ipairs(recs) do
  print(record.type)
end
```

Returns a lua table (sequence) containing all parsed JSON records in the batch.
Returns an empty table if the batch has already been committed.

### batch:iter_records

```lua
for record in batch:iter_records() do
  print(record.type)
end
```

Returns an iterator that yields one parsed JSON record at a time, converting
each record lazily on demand.  Returns an iterator that immediately yields
`nil` if the batch has already been committed.

Prefer this over `:records()` when processing large batches to avoid
materialising the entire batch into a table at once.

### batch:consumer_name

```lua
local name = batch:consumer_name()
```

Returns the name of the consumer this batch belongs to.  This is the `name`
field from the consumer configuration passed to
[`kumo.jsonl.new_multi_tailer`](new_multi_tailer.md).

For batches returned by the single-consumer
[`kumo.jsonl.new_tailer`](new_tailer.md), this returns `"default"`.

Returns an empty string if the batch has already been committed.

### batch:commit

```lua
batch:commit()
```

Advances the checkpoint to the end of this batch, confirming that the caller
has successfully processed all records in it.

If the tailer was not configured with a `checkpoint_name` this is a no-op.

Calling `:commit()` a second time on the same batch raises an error:
`"batch has already been committed"`.
