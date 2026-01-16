---
tags:
 - audit_series
---

{{since('dev')}}

# Audit_series

Audit_series maintains an in-memory chronological set of metrics of events. Metrics are not persisted across process restarts and are only meant for short-term bookkeeping of events. If persistence or data sharing across multiple kumomta instances is important, try using external database such as redis for storage.

## Available Methods { data-search-exclude }