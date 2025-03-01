# min_free_space

{{since('2024.09.02-c5476b89')}}

Specifies the desired minimum amount of free disk space for the spool storage
in this location.  Can be specified using either a string like `"10%"` to
indicate the percentage of available space, or a number to indicate the
number of available bytes.

If the available storage is below the specified amount then kumomta will
reject incoming SMTP and HTTP injection requests and the
[check-liveness](../../rapidoc.md/#get-/api/check-liveness/v1) endpoint will indicate
that new messages cannot be received.

The default value for this option is `"10%"`.


