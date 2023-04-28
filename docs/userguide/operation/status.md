# Getting Server Status

Once KumoMTA is installed, you can check on the server status with systemctl.
```console
sudo systemctl status kumomta
```
The result should look something like this:
``` kumomta.service - KumoMTA SMTP service
     Loaded: loaded (/lib/systemd/system/kumomta.service; enabled; vendor preset: enabled)
     Active: active (running) since Thu 2023-04-27 22:59:06 MST; 10h ago
   Main PID: 17912 (kumod)
      Tasks: 28 (limit: 19190)
     Memory: 257.1M
     CGroup: /system.slice/kumomta.service
             └─17912 /opt/kumomta/sbin/kumod --policy /opt/kumomta/etc/policy/init.lua --user kumod

Apr 27 22:59:06 kdev2.kumomta.com systemd[1]: Started KumoMTA SMTP service.
Apr 27 22:59:06 kdev2.kumomta.com kumod[17912]: 2023-04-28T05:59:06.444479Z  INFO main kumod::memory: using limits: soft=Some("12.58 GB"), hard=So>
Apr 27 22:59:06 kdev2.kumomta.com kumod[17912]: 2023-04-28T05:59:06.450824Z  INFO localset-2 kumod::http_server: http listener on 0.0.0.0:8000
Apr 27 22:59:06 kdev2.kumomta.com kumod[17912]: 2023-04-28T05:59:06.471926Z  INFO localset-2 kumod::spool: starting enumeration for meta
Apr 27 22:59:06 kdev2.kumomta.com kumod[17912]: 2023-04-28T05:59:06.471995Z  INFO localset-2 kumod::smtp_server: smtp listener on 0.0.0.0:25
Apr 27 22:59:06 kdev2.kumomta.com kumod[17912]: 2023-04-28T05:59:06.472008Z  INFO localset-2 kumod::smtp_server: smtp listener on 0.0.0.0:2026
Apr 27 22:59:06 kdev2.kumomta.com kumod[17912]: 2023-04-28T05:59:06.475882Z  INFO localset-2 kumod: initialization complete
```

The above is from a newer installation, but the log will grow.  If you are debugging an older install, `journalctl -r -n 10 -u kumomta.service` will show the last 10 lines (-n 10) in reverse order (-r).  `man journalctl` is your friend.


If you need to find the installed version, you can run `sudo /opt/kumomta/sbin/kumod --version`.  This will be important if you ever need to reach out for support.


If you have configured an HTTP listener, you can access server metrics with `curl -i 'http://localhost:8000/metrics'`. That will show a long form of the server metrics with detailed comments.  If you want just the data in a nice JSON format, use `curl -i 'http://localhost:8000/metrics.json`.

Metrics available are:

* connection_count: number of active connections

`connection_count{service="esmtp_listener"} 0`
* lua_count: the number of lua contexts currently alive

`lua_count 0`
* lua_load_count: how many times the policy lua script has been loaded into a new context

`lua_load_count 1`
* lua_spare_count: the number of lua contexts available for reuse in the pool

`lua_spare_count 0`
* memory_limit: soft memory limit measured in bytes

`memory_limit 12583633920`
* memory_usage: number of bytes of used memory

`memory_usage 277835776`


