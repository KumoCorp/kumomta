# Starting KumoMTA

If you followed all the instructions above without errors, you should now have a working MTA on a properly sized server.

Check to see if it is already running from the install instructions above:

```bash
sudo systemctl status kumomta
```

If the service is already running, you will need to restart it to apply the changes made above:

```bash
sudo systemctl restart kumomta
```

Start the MTA with this:

```bash
sudo systemctl start kumomta
```

You can enable it to restart as a service with a reboot with:

```bash
sudo systemctl enable kumomta
```

Alternately you can start it manually with:

```bash
 sudo /opt/kumomta/sbin/kumod --policy \
 /opt/kumomta/etc/policy/init.lua --user kumod&
```

* Using sudo allows KumoMTA to run as a privileged user so it can access port 25 which is needed to deliver via SMTP to the internet.
* The daemon `kumod` is the MTA.
* The directive --policy makes kumod load the 'init.lua' file as configuration policy.
* Because we launched with sudo, you need to use the directive --user and provide a valid user to assign responsibility to.
* The line ends with a `&` that forces the daemon to run in the background and returns you to a usable prompt (use `fg` to bring it back to the foreground).

You can also get immediate feedback by pre-pending ```KUMOD_LOG=kumod=info``` (or debug for more detail):

```bash
sudo KUMOD_LOG=kumod=info /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod&
```

If all goes well, it should return a PID and drop you back to a Linux prompt.

If KumoMTA does not start, refer to the [Troubleshooting Page](../userguide/operation/troubleshooting.md) of the User Guide.
