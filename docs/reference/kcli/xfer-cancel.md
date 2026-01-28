---
tags:
  - ops
  - xfer
---
# kcli xfer-cancel


Cancels a message transfer that was initiated via the xfer subcommand.  You specify the name of the xfer queue associated with the transfer and matching messages will be taken out of that queue and returned to their originating queue


**Usage:** `kcli xfer-cancel --reason <REASON> <QUEUE_NAME>`

## Arguments


* `<QUEUE_NAME>` — The name of the xfer queue that you wish to cancel

## Options


* `--reason <REASON>` — Each matching message will be rebound into its originating queue, and an AdminRebind log will be generated to trace that the rebind happened.  The reason you specify here will be included in that log record



