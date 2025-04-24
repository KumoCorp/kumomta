# Docker Swarm Example

This directory contains a basic configuration that will spawn
both kumod (the MTA process) and tsa-daemon (the traffic shaping
automation daemon), along with a supporting redis service to share
throttles, from a single docker compose file.

## Caveats

> [!CAUTION]
> *The compose file as shown here is not suitable for deploying
> directly into production*.
>
> While docker swarm allows you to scale the number of replicas
> defined in this stack, you need to consider the points below
> before attempting to deploy this in production, otherwise you
> risk loss of accountability of messages.

* The redis service that is part of this stack has no persistence
  and no redundancy. The built-in use of redis by kumomta is for tracking
  throttles, so the impact of a failure will be to the throttling and
  traffic shaping capability. It is non-trivial to setup a redundant
  redis cluster via docker swarm, so it is recommended that you
  deploy an appropriately provisioned redis cluster outside of this
  example swarm configuration.

* There is no persistent volume management in this example.  This means
  that draining, scaling or removing containers in the kumod service will
  lose whatever messages were stored in the associated spool. There is
  no simple out-of-the-box way to define persistent volume management with
  docker swarm, but you could consider deploying on something like
  glusterfs with per-slot spool directories. The
  [policy/init.lua](policy/init.lua) file included here is configured in such
  a way that you could define a shared NFS or gluster volume and mount
  it at `/var/spool/kumomta` in each kumod service instance.
  The performance characteristics of this kind of setup have not been
  tested or validated by the KumoMTA maintainers.

* Updating config files in docker swarm is relatively high friction

## Prerequisites

You must have a docker swarm configured. See [Swarm mode
overview](https://docs.docker.com/engine/swarm/) for more information.

## Starting the Stack

```console
$ docker stack deploy --compose-file compose.yaml kumomta
```

The service will spawn and bind to port 25 on the host system.

```console
$ docker stack ls
NAME      SERVICES
kumomta   3
$ docker service ls
ID             NAME            MODE         REPLICAS   IMAGE                                 PORTS
mufkyv9dp0th   kumomta_kumod   replicated   4/4        ghcr.io/kumocorp/kumomta:main   *:25->2525/tcp
ssee44osaugy   kumomta_redis   replicated   1/1        ghcr.io/kumocorp/redis:latest
zad4zz1unn5t   kumomta_tsa     replicated   2/2        ghcr.io/kumocorp/kumomta:main
```

You can then attempt to send mail on port 25, using swaks.

> [!IMPORTANT]
> Only domains configured in the
> [policy/listener_domains.toml](policy/listener_domains.toml) file will be
> relayed.

```
$ docker run --rm -it --net host nicolaka/netshoot \
    swaks -f user@example.com -t user@example.com \
    --server 0:25
```

## Tearing it down

```console
$ docker stack rm kumomta
```
