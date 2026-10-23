# IP warmup

IP warmup is the practice of gradually ramping volume on a new sending IP so mailbox providers can build a reputation history for it, typically over four to eight weeks from hundreds of messages per provider per day toward full volume. Each volume increase is gated on bounce rates, complaint rates, and reputation staying healthy. Sending full volume from a cold IP triggers throttling and blocks. Warmup schedules are provider-specific, and modern MTAs can automate the ramp.
