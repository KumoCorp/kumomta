# Back pressure

Back pressure is a system's ability to push resistance upstream when a downstream stage can't keep up. In email, that means the MTA slowing or refusing injection when queues are saturated, rather than accepting unbounded mail and collapsing. Well-designed sending pipelines propagate back pressure from the destination provider all the way to the injecting application.
