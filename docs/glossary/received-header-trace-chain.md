# Received header (trace chain)

Received headers are the trace lines each mail server prepends as a message passes through it, recording the handing-off host, the receiving host, the protocol, and a timestamp. Read from bottom to top, they reconstruct the message's full path, which makes them the primary forensic tool for debugging routing, latency, and forgery.
