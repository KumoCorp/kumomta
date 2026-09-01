# Deferral

A deferral is a temporary (4xx) refusal by a receiving server, "not now, try later," often used to enforce rate limits, apply greylisting, or signal reputation problems. Deferred messages return to the queue and retry on a schedule; a rising deferral rate is one of the earliest signals that a mailbox provider is throttling you.
