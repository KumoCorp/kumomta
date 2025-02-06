# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements

* DKIM signer TTLs can be now be expressed using duration strings like `"5
  mins"`. Previously you could only use the integer number of seconds.
* debian packages will now unmask kumod and tsa-daemon services as part
  of post installation.  Thanks to @cai-n! #331
* [memoize](../reference/kumo/memoize.md) now has an optional
  `invalidate_with_epoch` parameter that allows you to opt a specific cache
  into epoch-based invalidation.

## Fixes

* When using
  [kumo.dkim.set_signing_threads](../reference/kumo.dkim/set_signing_threads.md),
  some extraneous unused threads would be created.
