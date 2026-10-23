# Bounce processing

Bounce processing is the operational loop that consumes delivery failures: capturing bounces, classifying them, suppressing invalid addresses, and feeding the results back to list owners and monitoring. At high volume, unprocessed bounces mean escalating blocks; mature senders run bounce processing as a production pipeline in its own right.
