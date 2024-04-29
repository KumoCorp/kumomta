# How do I resolve a `Permission Denied` error?

If you encounter an error message similar to the following:

```shell
stack traceback:
        [C]: in local 'poll'
        [string "?"]:5: in function 'kumo.start_http_listener'
        [string "/opt/kumomta/etc/policy/init.lua"]:54: in function <[string "/opt/kumomta/etc/policy/init.lua"]:13>
caused by: Permission denied (os error 13)
```

The error is caused when the files referenced (*and/or* their containing directory) are not readable or writable by the `kumod` user. In the example above, this refers to TLS certificates used in the HTTP listener.

Check that all files referenced to in your `init.lua` file (including the init.lua file itself), as well as their parent directories, are readable to the `kumod` user. These directories should have `g+x` permissions.
