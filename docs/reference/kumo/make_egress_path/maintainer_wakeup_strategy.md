# maintainer_wakeup_strategy

{{since('dev')}}

Adjusts how aggressively the readyq maintainer task will be awoken
as messages are placed into the readyq.

Can have one of two values:

 * `"Aggressive"` - the default. Every attempt to place a message into the
   ready queue will cause the associated maintainer task to wakeup to assess
   whether more connections need to be established.

 * `"Relaxed"` - Each submission attempt will perform a quick approximation
   and assessment of the current connection count to decide whether the
   maintainer task needs to be signalled.  If the number of connections
   matches the ideal for the current queue size, then the maintainer will
   not be signalled and it will wakeup periodically to reassess the
   load.

The primary purpose of the readyq maintainer is to establish new outbound
connections based on the queue size.  A `"Relaxed"` setting will cause
a precise assessment of that state to occur less frequently, reducing
CPU overhead, but it may result in an increase in latency for
outbound traffic when conditions are bursty.

In earlier versions of KumoMTA this option did not exist, but the
behavior was equivalent to the `"Aggressive"` setting.


