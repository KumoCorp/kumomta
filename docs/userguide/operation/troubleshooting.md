# Troubleshooting KumoMTA

There are several things that can go wrong, especially when first installing KumoMTA. This page is intended to help with troubleshooting common issues.

!!!Note
        There are multiple ways to get help with KumoMTA, see the [How To Get Help](../general/get_help.md) page for more information.

## Using Swaks

When troubleshooting, it helps to eliminate external factors, including the injecting email infrastructure. We recommend using Swaks to perform test injections as it is known to act in an RFC compliant way when injecting messages. See the [Swaks Documentation](http://www.jetmore.org/john/code/swaks/latest/doc/ref.txt) for more information.

## Tracing Server Communications

When having issues with injecting messages, use the `kcli trace-smtp-server` command to receive an output of all communications between KumoMTA and the incoming client.

``` console
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]   0ns === Connected 2023-11-24 15:54:55.532224578 UTC
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]   0ns === conn_meta received_from="2xx.xxx.xx.xx:40422"
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]   0ns === conn_meta received_via="1xx.x.xxx.xx:587"
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]   0ns === conn_meta reception_protocol="ESMTP"
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]  17µs <-  220 kumomta.abcdef.com KumoMTA
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 182ms  -> EHLO kumomta.abcdef.com
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 183ms === smtp_server_ehlo: Ok
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 183ms <-  250-kumomta.abcdef.com Aloha kumomta.abcdef.com
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 183ms <-  250-PIPELINING
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 183ms <-  250-ENHANCEDSTATUSCODES
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 183ms <-  250 STARTTLS
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 588ms  -> STARTTLS
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587] 588ms <-  220 Ready to Start TLS
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]    1s  -> QUIT
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]    1s <-  221 So long, and thanks for all the fish!
[2xx.xxx.xx.xx:40422->1xx.x.xxx.xx:587]    1s === Closed
```

## Reviewing the System Journal

KumoMTA logs to the system journal for all error and status messages during operation, to view the log entries use journalctl:

```console
[root@localhost ~]# journalctl -f -n 50 -u kumomta.service
Oct 19 21:52:59 localhost systemd[1]: Started KumoMTA SMTP service.
Oct 19 21:53:00 localhost.localdomain kumod[902]: 2023-10-20T01:53:00.328546Z  INFO localset-0 kumod: NodeId is 2a32fb9b-7353-48bd-a06e-cc97e224c924
Oct 19 21:53:00 localhost.localdomain kumod[902]: 2023-10-20T01:53:00.337267Z  INFO localset-0 kumo_server_common::http_server: http listener on 127.0.0.1:8000
Oct 19 21:53:00 localhost.localdomain kumod[902]: 2023-10-20T01:53:00.348273Z  INFO localset-0 kumod::smtp_server: smtp listener on 0.0.0.0:25
Oct 19 21:53:01 localhost.localdomain kumod[902]: 2023-10-20T01:53:01.221127Z  INFO localset-0 kumod::spool: start_spool: enumeration done, spooled in 2 msgs over 117.40671ms
Oct 19 21:53:01 localhost.localdomain kumod[902]: 2023-10-20T01:53:01.221509Z  INFO localset-0 kumo_server_common::start: initialization complete
```

In this example the **-f** option tells journalctl to follow the log, in other words to tail or continuously read the file, the **-n 50** option tells journalctl to start by reading the previous 50 lines, and the **-u** option tells journalctl to filter by a specific unit, in this case the *kumomta.service* unit.

A common issue with new installs is ownership of the spool directory. When the spool is provisioned as a separate volume, it will not be owned by the **kumod** user. In this example we change ownership of the */var/spool/kumomta* directory, then attempt to start the kumomta service, then read the system journal to identify the issue:

```console
[root@localhost spool]# systemctl stop kumomta
[root@localhost spool]# chown -R root /var/spool/kumomta/
[root@localhost spool]# systemctl start kumomta
[root@localhost spool]# journalctl -f -n 50 -u kumomta.service
Oct 19 22:09:06 localhost.localdomain systemd[1]: Started KumoMTA SMTP service.
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.752782Z  INFO localset-0 kumod: NodeId is 2a32fb9b-7353-48bd-a06e-cc97e224c924
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.755699Z  INFO localset-0 kumo_server_common::http_server: http listener on 127.0.0.1:8000
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.756982Z  INFO localset-0 kumod::smtp_server: smtp listener on 0.0.0.0:25
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.757415Z ERROR localset-0 kumod::spool: Error in spool: Opening spool data: opening pid file /var/spool/kumomta/data/lock: Permission denied (os error 13)
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.758039Z ERROR localset-0 kumod::spool: Error in spool: Opening spool meta: opening pid file /var/spool/kumomta/meta/lock: Permission denied (os error 13)
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.758363Z  INFO localset-0 kumod::smtp_server: smtp listener on 0.0.0.0:25 -> stopping
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.758051Z  INFO       main kumo_server_common::start: Shutdown completed OK!
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.772671Z ERROR localset-0 kumo_server_common::start: problem initializing: No spools have been defined
Oct 19 22:09:06 localhost.localdomain kumod[5356]: 2023-10-20T02:09:06.772827Z  INFO localset-0 kumo_server_common::start: initialization complete
Oct 19 22:09:06 localhost.localdomain kumod[5356]: Error: Initialization raised an error
Oct 19 22:09:06 localhost.localdomain systemd[1]: kumomta.service: Main process exited, code=exited, status=1/FAILURE
Oct 19 22:09:06 localhost.localdomain systemd[1]: kumomta.service: Failed with result 'exit-code'.
Oct 19 22:09:07 localhost.localdomain systemd[1]: Failed to start KumoMTA SMTP service.
```

This error message makes it clear that there was an issue with permissions on the spool folder that prevented the kumomta service from starting.

## Changing the Log Level

Sometimes the default logging level will not expose sufficient information to troubleshoot certain issues.

To increase the verbosity of the logs written to the system journal, use the [kumo.set_diagnostic_log_filter](../../reference/kumo/set_diagnostic_log_filter.md) function in your `init.lua`` policy's **init** event handler:

```lua
kumo.on('init', function()
  kumo.set_diagnostic_log_filter 'kumod=debug'
end)
```

In addition, you can adjust the log filter level dynamically [using the HTTP API](../../reference/http/api_admin_set_diagnostic_log_filter_v1.md):

```console
curl -i 'http://localhost:8000/api/admin/set_diagnostic_log_filter/v1' \
    -H 'Content-Type: application/json' \
    -d '{"filter":"kumod=debug"}'
```

This will produce output similar to the following:

```console
Oct 20 09:26:43 localhost.localdomain systemd[1]: Started KumoMTA SMTP service.
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.030934Z  INFO localset-2 kumod: NodeId is 2a32fb9b-7353-48bd-a06e-cc97e224c924
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.032892Z  INFO localset-2 kumod::smtp_server: smtp listener on 0.0.0.0:25
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.033051Z DEBUG localset-2 kumod::spool: Defining local disk spool 'data' on /var/spool/kumomta/data
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.033179Z ERROR localset-2 kumod::spool: Error in spool: Opening spool data: opening pid file /var/spool/kumomta/data/lock: Permission denied (os error 13)
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.033404Z DEBUG localset-2 kumod::spool: Defining local disk spool 'meta' on /var/spool/kumomta/meta
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.033490Z ERROR localset-2 kumod::spool: Error in spool: Opening spool meta: opening pid file /var/spool/kumomta/meta/lock: Permission denied (os error 13)
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.048202Z  INFO localset-2 kumod::smtp_server: smtp listener on 0.0.0.0:25 -> stopping
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.050200Z DEBUG     logger kumod::logging: started logger thread
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.051984Z DEBUG     logger kumod::logging: calling state.logger_thread()
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.052337Z DEBUG     logger kumod::logging: LogFileParams: LogFileParams {
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     log_dir: "/var/log/kumomta",
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     max_file_size: 1000000000,
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     back_pressure: 128000,
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     compression_level: 0,
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     max_segment_duration: None,
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     meta: [],
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     headers: [],
Oct 20 09:26:44 localhost.localdomain kumod[6061]:     per_record: {},
Oct 20 09:26:44 localhost.localdomain kumod[6061]: }
Oct 20 09:26:44 localhost.localdomain kumod[6061]: Error: Initialization raised an error
Oct 20 09:26:44 localhost.localdomain kumod[6061]: 2023-10-20T13:26:44.053482Z DEBUG     logger kumod::logging: waiting until deadline=None for a log record
Oct 20 09:26:44 localhost.localdomain systemd[1]: kumomta.service: Main process exited, code=exited, status=1/FAILURE
Oct 20 09:26:44 localhost.localdomain systemd[1]: kumomta.service: Failed with result 'exit-code'.
```

Note the additional DEBUG level log entries compared to the previous example.

The log levels available, in order from least to most verbose are:
* Error
* Warn
* Info
* Debug
* Trace

!!!warning
    The lower, more verbose levels of log levels can be very verbose, especially the  **trace** level. These levels should not be enabled permanently as they can lead to a full disk in a short period of time.
