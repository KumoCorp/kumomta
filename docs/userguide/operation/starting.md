# Starting KumoMTA

Once KumoMTA has been installed and an initial policy script is in place, the
server can be started with the following command:

```console
$ sudo systemctl start kumomta
```

It can be enabled as a service with:

```console
$ sudo systemctl enable kumomta
```

You can check the status with:

```console
$ sudo systemctl status kumomta
```

It can also be started manually with:

```console
$ sudo /opt/kumomta/sbin/kumod \
    --policy /opt/kumomta/etc/policy/init.lua \
    --user kumod
```

* Using sudo allows it to run as a privileged user so it can access port 25 which is needed to send and receive from most MTAs.
* The daemon `kumod` is the MTA
* The directive --policy makes kumod load the 'init.lua' file as configuration policy.
* The *--user* directive allows the server to drop privileges after attaching to port 25 so that it does not continue to run as root.

For more detailed output, prepend ```KUMOD_LOG=kumod=info``` (or debug for even more detail):

```console
$ sudo KUMOD_LOG=kumod=info /opt/kumomta/sbin/kumod \
   --policy /opt/kumomta/etc/policy/init.lua --user kumod
```

If all goes well, it should return a PID and drop you back to a Linux prompt.

If KumoMTA does not start, refer to the [Troubleshooting Page](./troubleshooting.md) of the User Guide.
