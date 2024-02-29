# redis + redis-cell module

This directory contains a Dockerfile that will build a redis docker image that
includes [redis-cell](https://github.com/brandur/redis-cell), a redis module
that provides a `CL.THROTTLE` command that employs a [generic cell rate
algorithm](https://en.wikipedia.org/wiki/Generic_cell_rate_algorithm) to
provide throttling functionality.

## Pre-built Image

The KumoMTA CI builds out a redis docker image that includes the redis-cell
module.  The image supports amd64, arm64 and armv7 architectures.

You can start it like this:

```console
$ docker run --name kumomta-redis -p 6379:6379 -d ghcr.io/kumocorp/redis
```

## Build

To build the image for yourself:

```console
$ docker build -t redis-cell .
```

To run it:

```console
$ docker run --name redis -p 6379:6379 -d redis-cell
```

