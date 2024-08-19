# timerwheel_tick_interval

{{since('dev')}}

When using the default `strategy = "TimerWheel"`, the timer wheel needs to
be ticked regularly in order to promote messages into the ready queue. The default
tick interval is computed as `retry_interval / 20` and clamped to be within the
range `>= 1s && <= 1m`.

If you have a short `retry_interval` and a lot of scheduled queues you may find
that your system is spending more time ticking over than is desirable, so you can
explicitly select the tick interval via this option.

The value is an optional string duration like `1m`.

If you have to set this, our recommendation is generally for this to be as long
as possible.

!!! note
    The maintainer will also tick over whenever the
    [refresh_interval](refresh_interval.md) elapses, so there isn't a tangible
    benefit to setting `timerwheel_tick_interval` larger than `refresh_interval`.

