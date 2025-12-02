# kumo.version

{{since('2025.12.02-67ee9e96')}}

This constant is set to the kumo version string that is also reported by
running `kumod --version`. This can potentially be used to adjust configuration
according to the installed/running version.

The version string looks like `2025.11.09-d4028f99`. You can compare the
strings lexicographically if you wish to test whether a given version is newer
than another; the first component is the date on which the release was made,
the second component component is a git hash.

```lua
print(kumo.version)
-- prints something like: 2025.11.09-d4028f99
```


