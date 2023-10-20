# Troubleshooting KumoMTA

There are several things that can go wrong, especially when first installing KumoMTA. This page is intended to help with troubleshooting common issues.

!!!Note
        There is a community Discord server available at https://kumomta.com/discord where you can ask for community assistance.

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

## Starting KumoMTA in the Foreground

When there are issues with starting KumoMTA that are not exposed through the system journal, an additional troubleshooting step is starting KumoMTA in the foreground with extended error reporting.

To run KumoMTA in the foreground with additional error logging use the following command:

```console
sudo KUMOD_LOG=kumod=debug /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod
```

This will produce output similar to the following:

```console
[root@localhost spool]# sudo KUMOD_LOG=kumod=debug /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod
2023-10-20T02:15:00.481840Z  INFO localset-0 kumod: NodeId is 2a32fb9b-7353-48bd-a06e-cc97e224c924
2023-10-20T02:15:00.485934Z  INFO localset-0 kumod::smtp_server: smtp listener on 0.0.0.0:25
2023-10-20T02:15:00.486788Z DEBUG localset-0 kumod::spool: Defining local disk spool 'data' on /var/spool/kumomta/data
2023-10-20T02:15:00.487283Z ERROR localset-0 kumod::spool: Error in spool: Opening spool data: opening pid file /var/spool/kumomta/data/lock: Permission denied (os error 13)
2023-10-20T02:15:00.488043Z DEBUG localset-0 kumod::spool: Defining local disk spool 'meta' on /var/spool/kumomta/meta
2023-10-20T02:15:00.488177Z ERROR localset-0 kumod::spool: Error in spool: Opening spool meta: opening pid file /var/spool/kumomta/meta/lock: Permission denied (os error 13)
2023-10-20T02:15:00.488552Z  INFO localset-0 kumod::smtp_server: smtp listener on 0.0.0.0:25 -> stopping
2023-10-20T02:15:00.514056Z DEBUG     logger kumod::logging: started logger thread
2023-10-20T02:15:00.515102Z DEBUG     logger kumod::logging: calling state.logger_thread()
Error: Initialization raised an error
```

One again we have a clear indication that there is an issue with permissions on the spool directory.
