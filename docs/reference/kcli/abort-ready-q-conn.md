---
tags:
  - ops
  - debugging
---
# kcli abort-ready-q-conn


Aborts the dispatcher task within a ready queue identified by its session_id. The dispatcher's drop path returns any in-flight message to the scheduled queue for another delivery attempt


**Usage:** `kcli abort-ready-q-conn <QUEUE_NAME> <SESSION_ID>`

## Arguments


* `<QUEUE_NAME>` — The name of the ready queue

* `<SESSION_ID>` — The session_id of the dispatcher to abort. Obtain this via `kcli inspect-ready-q`



