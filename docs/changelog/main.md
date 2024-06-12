# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
* The [kumomta-dev container
  image](https://github.com/KumoCorp/kumomta/pkgs/container/kumomta-dev) is now
  a multiarch image, supporting both `linux/amd64` and `linux/arm64`
  architectures.  Simply use `docker pull ghcr.io/kumocorp/kumomta-dev:latest`
  to get the appropriate architecture.

## Fixes
* Using `expiration` in a DKIM signer would unconditionally raise an error and
  prevent reception of the incoming message.

