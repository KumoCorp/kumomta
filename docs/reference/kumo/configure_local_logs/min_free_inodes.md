---
tags:
 - logging
---
# min_free_inodes

{{since('2024.09.02-c5476b89')}}

Specifies the desired minimum amount of free inodes for the log storage
in this location.  Can be specified using either a string like `"10%"` to
indicate the percentage of available inodes, or a number to indicate the
number of available inodes.

If the available inodes are below the specified amount then kumomta will
reject incoming SMTP and HTTP injection requests and the
[check-liveness](../../http/kumod/api_check_liveness_v1_get.md) endpoint will indicate
that new messages cannot be received.

The default value for this option is `"10%"`.


