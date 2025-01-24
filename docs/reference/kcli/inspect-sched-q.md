# kcli inspect-sched-q


Returns information about a scheduled queue.

Part of the information is a sample of the messages contained within that queue.

Depending on the configured queue strategy, it may not be possible to sample messages from the queue. At the time of writing, the server side can only provide message information if the strategy is set to "SingletonTimerWheel" (the default).


**Usage:** `kcli inspect-sched-q [OPTIONS] <QUEUE_NAME>`

## Arguments


* `<QUEUE_NAME>` — The name of the queue that you want to query

## Options


* `--want-body` — Whether to include the message body information in the results. This can be expensive, especially with large or no limits

* `--limit <LIMIT>` — How many messages to include in the sample. The default is 5 messages. The messages are an unspecified subset of the messages in the queue and likely do NOT indicate which message(s) will be next due for delivery

    Default value: `5`

* `--no-limit` — Instead of guessing at a limit, run with no limit on the number of messages returned.  This can be expensive, especially if `--want-body` is also enabled



