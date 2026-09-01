# IP reputation vs domain reputation

IP reputation attaches to the sending address; domain reputation attaches to the domains in the message (From, DKIM, links). Modern filtering weights domain reputation heavily, and you cannot outrun a bad domain by rotating IPs, but IP reputation still gates connection acceptance and rate limits at most providers.
