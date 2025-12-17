# Unreleased Changes in The Mainline

## Breaking Changes

## Other Changes and Enhancements
 * The `Handlebars` template dialect now runs with recursive lookup
   for improved compatibility with other handlebars implementations.

## Fixes

 * An SPF record containing U+200B (zero width space) could cause
   SPF record parsing to panic and the service to crash

