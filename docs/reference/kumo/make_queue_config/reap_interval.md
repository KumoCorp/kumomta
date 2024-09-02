# reap_interval

{{since('2024.09.02-c5476b89')}}

Optional duration string. The default is `"10m"`.  It controls how long the
queue should remain empty and idle before we reap it from the queue management
layer and free its associated resources and metrics.


