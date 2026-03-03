---
tags:
  - ops
---
# kcli queue-summary


Prints a summary of the state of the queues, for a human to read.

Note that this output format is subject to change and is not suitable for a machine to parse. It is expressly unstable and you must not depend upon it in automation.

The data behind this output is pulled from the metrics endpoint, which is machine readable.

The output is presented in two sections:

1. The ready queues

2. The scheduled queues

The ready queue data is presented in columns that are mostly self explanatory, but the numeric counts are labelled with single character labels:

D - the total number of delivered messages

T - the total number of transiently failed messages

C - the number of open connections

Q - the number of ready messages in the queue

Note that the ready queue counter values reset whenever the ready queue is reaped, which occurs within a few minutes of the ready queue being idle, so those numbers are only useful to get a sense of recent/current activity. Accurate accounting must be performed using the delivery logs and not via this utility.

The scheduled queue data is presented in two columns; the queue name and the number of messages in that queue.


**Usage:** `kcli queue-summary [OPTIONS]`

## Options


* `--limit <LIMIT>` — Limit results to LIMIT results

* `--by-volume` — Instead of ordering by name, order by volume, descending

* `--domain <DOMAIN>` — Filter queues to those associated with a DNS domain



