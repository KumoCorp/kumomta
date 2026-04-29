# Module `kumo.jsonl`

{{since('2026.04.09-ea3b2a9b')}}

This module provides utilities for reading and writing
[JSONL](https://jsonlines.org/) (newline-delimited JSON) log files stored in
zstd-compressed segments, as produced by kumomta's logging subsystem.

It supports:

* **Writing** — [`kumo.jsonl.new_writer`](new_writer.md) produces a
  `LogWriter` that compresses records into time-based segment files.

* **Reading (single consumer)** — [`kumo.jsonl.new_tailer`](new_tailer.md)
  returns a streaming tailer for a single consumer with optional filtering,
  checkpointing, and configurable batching.

* **Reading (multiple consumers)** —
  [`kumo.jsonl.new_multi_tailer`](new_multi_tailer.md) fans a single
  read pass out to multiple independent consumers, each with its own filter,
  batch parameters, and checkpoint.

## Available Functions { data-search-exclude }
