# Unreleased Changes in The Mainline

## Breaking Changes

* Site-name generation now preserves complete hostname branches instead of
  combining labels independently. Distinct MX sets that previously collided
  now have separate site names and ready queues. For example, two real-world
  Zoho MX sets:

  ```text
  Set A: mx.zoho.com, mx2.zoho.com, mx3.zoho.eu
  Set B: mx.zoho.com, mx2.zoho.eu,  mx3.zoho.com

  Before (both): (mx|mx2|mx3).zoho.(com|eu)
  After A:       ((mx|mx2).zoho.com|mx3.zoho.eu)
  After B:       ((mx|mx3).zoho.com|mx2.zoho.eu)
  ```

  Site names are also independent of record order and MX preference. Some existing
  site names change. Review hard-coded site-name configuration matches, metric
  series and site-keyed TSA state when upgrading. See
  [Site Names](../reference/queues.md#site-names) for details.

## Other Changes and Enhancements

## Fixes

