# redis + redis-cell module

This directory contains a Dockerfile that will build a redis docker image that
includes [redis-cell](https://github.com/brandur/redis-cell), a redis module
that provides a `CL.THROTTLE` command that employs a [generic cell rate
algorithm](https://en.wikipedia.org/wiki/Generic_cell_rate_algorithm) to
provide throttling functionality.

## Build

To build the image:

```bash
docker build -t redis-cell .
```

To run it:

```bash
docker run --name redis -p 6379:6379 -d redis-cell
```
