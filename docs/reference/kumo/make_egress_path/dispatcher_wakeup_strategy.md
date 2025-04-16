# maintainer_wakeup_strategy

{{since('dev')}}

Adjusts how aggressively the readyq Dispatch tasks will be awoken
as messages are placed into the readyq.

Can have one of two values:

 * `"Aggressive"` - the default. Every attempt to place a message
   into the ready queue will cause all idle outbound sessions
   that are associated with the readyq to wakeup and attempt to
   pull messages from it.

 * `"Relaxed"` - Each submission attempt will signal just a single
   idle outbound session to wakeup and pull a message from the
   queue.

An `"Aggressive"` setting will cause more spurious wakeups in
the case that there are multiple sessions and just a single new
message being submitted, whereas a `"Relaxed"` setting will be
more optimal in terms of CPU usage, at the risk of potentially
missing a wakeup in some edge cases with bursty or low traffic.

In earlier versions of KumoMTA this option did not exist, but the
behavior was equivalent to the `"Aggressive"` setting.
