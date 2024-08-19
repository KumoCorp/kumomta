# egress_pool

The name of the egress pool which should be used as the source of
this traffic.

If you do not specify an egress pool, a default pool named `unspecified`
will be used. That pool contains a single source named `unspecified` that
has no specific source settings: it will just make a connection using
whichever IP the kernel chooses.

See [kumo.make_egress_pool()](../make_egress_pool/index.md).


