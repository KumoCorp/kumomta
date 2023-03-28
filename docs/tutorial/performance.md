# Tuning for Performance

You can get better performace with some fine tuning of system parameters,  The settings below are examples only but have worked in test and development servers.  As the saying goes, "your milage may vary" so you should research these and tune as needed for your own system.

KumoMTA makes heavy use of files, RAM, CPU and network resources. Setting these can be helpful as a default Linux install assumes the need to share resources with many applications, but we need to allow KumoMTA to use as much of the resource pool as possible.

```console
# Tune sysctl setings. Note that these are suggestions,
#  you should tune according to your specific build

echo "
vm.max_map_count = 768000
net.core.rmem_default = 32768
net.core.wmem_default = 32768
net.core.rmem_max = 262144
net.core.wmem_max = 262144
fs.file-max = 250000
net.ipv4.ip_local_port_range = 5000 63000
net.ipv4.tcp_tw_reuse = 1
kernel.shmmax = 68719476736
net.core.somaxconn = 1024
vm.nr_hugepages = 20
kernel.shmmni = 4096
" | sudo tee -a /etc/sysctl.conf > /dev/null

sudo /sbin/sysctl -p |sudo tee -a /etc/sysctl.conf
```

## Performance Test
OK, now lets really test this with some volume.  You will **_not_** want to do that in the public internet with real adresses for a number of reasons, so you should set up another KumoMTA instance and have it run the included "sink.lua" policy.  That will set KumoMTA to accept all messages and discard them without forwarding.

Once you have the second one configured, you can create a script on the sending server to loop over an injection to the sink as many time as you like. Using a function like 'time' can give you an accurate time windows and knowing the number of loops and size of email, you can determine performance. 

Part of the packaged build is a "traffic Generator" that can be use to send volume test mail for this purpose.  It will append a known domain to all outbound mail that resolves to your own loopback address so that mail can be delivered, but will never deliver to real addresses.  You can use it like this:
```console
sudo /opt/kumomta/sbin/traffic-gen --target <your.sink.server>:587 --concurrency 20000 --message-count 100000 --body-size 100000 
```

Get more usage info with 
```console
sudo /opt/kumomta/sbin/traffic-gen --help
```

That traffic generator was used to test throughput before the initial release. There is a sample chart below of one done recently.

One handy setting for performance testing is [custom routing](https://docs.kumomta.com/userguide/policy/routing/).  If you configure your sending server to route all messages to the sink server, and the sink is configured to dev/null all messages, that can make your testing script less complicated.  IE:

```console
kumo.on('smtp_server_message_received', function(msg)
    msg:set_meta('queue', 'my.sink.server')
end)

```

## Test Results
The test configuration was one "sending" configured KumoMTA in AWS (variable CPU and RAM) and one "sink" KumoMTA in Azure (8 CPU/16GB RAM) using a payload of 100KB sent in a loop 100,000 times. 
The test usilized the included traffic-gen utility described above.

| CPU | RAM | RATE |
| --- | --- | --- |
| 2   | 4  |  2.7 MMH  |
| 4   | 16  | 4.4 MMH  |
| 8   | 30  | 4.9 MMH  |
| 16   | 64  | 5.1 MMH  |

**NOTE** that these numbers are NOT guaranteed and are for informational purposes only. Your results may vary considerably.


## Now What?

RTFM.  Seriously.  KumoMTA is a very powerful, highly configurable MTA that you can integrate in many ways.  There is no way we can document every possible use case or configuration.


