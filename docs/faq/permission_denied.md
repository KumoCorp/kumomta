# How do I resolve a `Permission Denied` error?

If you encounter an error message similar to the following:

```shell
2023-10-30T22:08:46.109744Z DEBUG     logger kumod::logging: waiting until deadline=None for a log record
2023-10-30T22:08:46.111633Z DEBUG       main kumod::logging: Terminating a logger
2023-10-30T22:08:46.111661Z DEBUG       main kumod::logging: Joining that logger
2023-10-30T22:08:46.111679Z DEBUG     logger kumod::logging: LogCommand::Terminate received. Stopping writing logs
2023-10-30T22:08:46.111804Z DEBUG     logger kumod::logging: Clearing any buffered files prior to completion
2023-10-30T22:08:46.112028Z DEBUG       main kumod::logging: Joined -> Some(Ok(()))
Error: Initialization raised an error: call init callback: callback error
stack traceback:
        [C]: in local 'poll'
        [string "?"]:5: in function 'kumo.start_http_listener'
        [string "/opt/kumomta/etc/policy/init.lua"]:54: in function <[string "/opt/kumomta/etc/policy/init.lua"]:13>
caused by: Permission denied (os error 13)
```

The errror is caused when the files referenced (*and/or* their containing directory) is not readable or writable by the `kumod` user.

Check that all files referenced in your `init.lua` (including the init.lua) file, as well as their parent directories, are readable to the `kumod` user.