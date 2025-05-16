# kumo.configure_tsa_db_path

```lua
kumo.configure_tsa_db_path(PATH)
```

{{since('2023.08.22-4d895015')}}

This function should be called only from inside your
[tsa_init](../events/tsa_init.md) event handler, and *MUST* be called before
[kumo.tsa.start_http_listener](start_http_listener.md).

Its purpose is to specify the path in which the tsa-daemon will persist event
and configuration information in a local sqlite database.

The default value for `PATH` is `"/var/spool/kumomta/tsa.db"`.

Since the path is passed to sqlite, you may use URI filenames as specified in
the [sqlite3_open](https://www.sqlite.org/c3ref/open.html) documentation, such
as `":memory:"` to use an in-memory database that will be discarded when the
tsa-daemon is restarted.


