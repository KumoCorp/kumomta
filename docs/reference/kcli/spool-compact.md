---
tags:
  - ops
  - debugging
---
# kcli spool-compact


Forces a flush and full compaction of the named spool.

Primarily a diagnostic and test helper.  For rocksdb-backed spools this calls flush() followed by a full-keyspace compact_range(). For other spool kinds it is a no-op.

If the underlying storage reports an error during the operation (for example, a missing or corrupt SST file in a rocksdb spool), the error is reported to the caller and the command exits non-zero.


**Usage:** `kcli spool-compact --name <NAME>`

## Options


* `--name <NAME>` — Name of the spool to compact, matching a name passed to `kumo.define_spool` in the policy



