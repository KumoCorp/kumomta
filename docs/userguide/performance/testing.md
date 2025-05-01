# Performance Testing
Performance testing **must not** be performed against the public internet, as large volumes of test message can be catastrophic for sending reputation. Instead, a second instance of KumoMTA should be installed that uses the sink.lua policy script found at [https://github.com/KumoCorp/kumomta/blob/main/sink.lua](https://github.com/KumoCorp/kumomta/blob/main/sink.lua).

Write the script to /opt/kumomta/etc/policy/sink.lua and start the sink server using the following command:

```bash
sudo KUMOD_LOG=kumod=info /opt/kumomta/sbin/kumod \
   --policy /opt/kumomta/etc/policy/sink.lua --user kumod
```

With the sink server configured and running, you can send test messages to the sink, knowing that they will be discarded and not relayed to the public Internet. You may want to block outbound traffic on port 25 from your testing servers to help ensure no messages are relayed externally.

Included with the packaged KumoMTA builds is a "Traffic Generator" that can be use to send volume test mail for this purpose. The `traffic-gen` appends a known domain to all outbound mail that resolves to your own loopback address so that mail can be delivered, but will never deliver to real addresses:

```bash
sudo /opt/kumomta/sbin/traffic-gen --target <your.sink.server>:25 --concurrency 20000 --message-count 100000 --body-size 100000
```

For additional parameters for the `traffic-gen` utility see:
```bash
sudo /opt/kumomta/sbin/traffic-gen --help
```

The `traffic-gen` script is used internally by the KumoMTA team to test performance before each release.

It is helpful to use [custom routing](https://docs.kumomta.com/userguide/policy/routing/) to configure the test server to route all messages to the sink server, with the sink configured to dev/null all messages. Modify the `init.lua` on the test server with the following:

```bash
kumo.on('smtp_server_message_received', function(msg)
    msg:set_meta('queue', 'my.sink.server')
end)
```

## Sample Test Results
The hardware configuration used in this example is one "sending" configured KumoMTA instance hosted on AWS (variable CPU and RAM) and one "sink" KumoMTA instance hosted on Azure (8 CPU/16GB RAM) using a payload of 100KB messages sent in a loop 100,000 times.

The test utilized the included traffic-gen utility as described above.

| CPU | RAM | RATE |
| --- | --- | --- |
| 2   | 4  |  2.7 MMH  |
| 4   | 16  | 4.4 MMH  |
| 8   | 30  | 4.9 MMH  |
| 16   | 64  | 5.1 MMH  |

**NOTE** that these numbers are NOT guaranteed and are for informational purposes only. Your results may vary considerably.

In July 2023 another round of testing was done with more detailed results.  those results are shown in the table below and were documented in the blog post [How we built the most performant Message Transfer Agent on the planet](https://kumomta.com/blog/building-the-fastest-mta-on-the-planet):

![PerformanceTestData](https://docs.kumomta.com/assets/images/Performance_testing_kumomta_public.png)
