# Single Node Docker Example

This directory contains a basic configuration that will spawn
both kumod (the MTA process) and tsa-daemon (the traffic shaping
automation daemon) from a single docker compose file.

Use this to start the daemons and view the logs:

```console
$ docker compose up
```

The service will spawn and bind to port 25 on the host system.

You can then send mail on port 25, using swaks:

```
$ docker run --rm -it --net host nicolaka/netshoot \
    swaks -f user@example.com -t user@example.com \
    --server $HOSTNAME:25
```

Trace submissions in realtime:

```
$ docker exec -t kumod kcli trace-smtp-server
```

