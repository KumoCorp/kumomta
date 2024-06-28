# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
* The [kumomta-dev container
  image](https://github.com/KumoCorp/kumomta/pkgs/container/kumomta-dev) is now
  a multiarch image, supporting both `linux/amd64` and `linux/arm64`
  architectures.  Simply use `docker pull ghcr.io/kumocorp/kumomta-dev:latest`
  to get the appropriate architecture.
* New [kumo.regex](../reference/regex/index.md) and
  [kumo.string](../reference/string/index.md) lua modules. #220
* New `kcli rebind` and
  [/api/admin/rebind/v1](../reference/rapidoc.md#post-/api/admin/rebind/v1) HTTP
  endpoint to allow moving/rebinding messages from one scheduled queue to
  another. There is an optional corresponding
  [rebind_message](../reference/events/rebind_message.md) event for more
  advanced rebinding logic. #209
* Moved JSON and TOML functions into a new
  [kumo.serde](../reference/kumo.serde/index.md) module. Those functions are
  also still available under the `kumo` module for backwards compatibility
  sake, but will be removed in a future release. You should standardize on the
  new `kumo.serde` module name moving forwards.
* Added YAML serialization/deserialization functions to
  [kumo.serde](../reference/kumo.serde/index.md).
* You may now run `kumod --validate` to perform extended validation checks
  of the helper configuration in your policy. This can be performed offline/concurrently
  with a running kumod. The output is human readable. The exit code will
  be 0 when no validation errors are detected, non-zero otherwise. #211

## Fixes
* Using `expiration` in a DKIM signer would unconditionally raise an error and
  prevent reception of the incoming message.
* Invalid structured headers, such as Message-ID, in combination with other message
  body conformance issues could cause
  [msg:check_fix_conformance](../reference/message/check_fix_conformance.md) to
  raise an error instead of fixing the issue. #216
* Swapped retry-after/reset-after results, and increased timestamp precision
  when using [cluster-backed
  throttles](../reference/kumo/configure_redis_throttles.md). Thanks to @cai-n!
  #217
