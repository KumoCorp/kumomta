# Performance Testing
Performance testing **must not** be performed against the public internet, as large volumes of test message can be catastrophic for sending reputation. This isn't to say that one-off test messages will be a problem, but that sending in bulk can cause serious issues.

When testing you should send against a mail sink server, but your choice of sink can significantly impact your testing results. It is very common to install something link smtp_sink from Postfix for testing, but many message sinks simply accept all messages and discard them. This results in zero backpressure or queue buildup on the MTA, which can lead to inaccurate results.

## Using KumoMTA's Smart Sink Docker Container
To ensure that your testing reflects the real world as much as possible, we recommend you use the *Smart Sink* Docker container found at [https://github.com/KumoCorp/kumomta/tree/main/examples/smart-sink-docker](https://github.com/KumoCorp/kumomta/tree/main/examples/smart-sink-docker) for testing. The Smart Sink policy will accept most mail and discard them, but it will respond to a configurable percentage of traffic with temporary and permanent failure messages that are appropriate for the destination domain of the message (for example, a message sent to yahoo.com that is flagged for a temporary failure will result in a temporary failure message used in production by Yahoo).

The Smart Sink can also recognize when the user portion of the email is `tempfail@` or `permfail@` and respond with a corresponding temporary or permanent failure message for the domain in the recipient address.

The setting for bounce and tempfail percentages, as well as potential responses, can be found at [https://github.com/KumoCorp/kumomta/blob/main/examples/smart-sink-docker/policy/responses.toml](https://github.com/KumoCorp/kumomta/blob/main/examples/smart-sink-docker/policy/responses.toml).

For instructions on deploying the Smart Sink Docker container, see [https://github.com/KumoCorp/kumomta/tree/main/examples/smart-sink-docker](https://github.com/KumoCorp/kumomta/tree/main/examples/smart-sink-docker).

## Generating Traffic
We strongly recommend deploying a QA version of your production traffic generating system for testing and loading it with data that closely mimics production data (ideally obfuscated production data). When performance testing the goal is to duplicate a production environment and workload as closely as possible, so you will want to generate the same volume of mail for the same variety of destination domains across the same number of IPs when possible to ensure that your test environment behaves as closely to your production environment as possible.

This is important because KumoMTA works in a highly parallel fashion, with very granular queues created for the various combinations of campaign, tenant, destination domain, egress_source, and site_name required by your outgoing traffic, and if you send your tests from a single tenant or to a single destination you will not be able to tune your environment for production traffic because you will have a small handful of queues instead of thousands of queues and the server behavior will be very different.

## Generating Traffic Using `traffic-gen`
For cases where accurate simulation is not feasible, KumoMTA includes a "Traffic Generator" that can be use to send volume test mail for this purpose. The `traffic-gen` appends a known domain to all outbound mail that resolves to your own loopback address so that mail can be delivered, but will never deliver to real addresses:

```console
$ /opt/kumomta/sbin/traffic-gen --target <your.sink.server>:25 --concurrency 20000 --message-count 100000 --body-size 100000
```

For additional parameters for the `traffic-gen` utility see:

```console
$ /opt/kumomta/sbin/traffic-gen --help
```

The `traffic-gen` script is used internally by the KumoMTA team to test performance before each release.

A more advanced example is as follows:

```console
$ cat traffic-gen.sh
#!/bin/bash

exec traffic-gen \
        --target localhost:2025 \
        --keep-going \
        --concurrency 400 \
        --body-size 60kb \
        --duration 1500 \
        --domain-suffix 'testingdomain.tld' \
        --domain aol.com:1.5 \
        --domain bellsouth.net:0.2 \
        --domain comcast.net:0.5 \
        --domain gmail.com:67.4 \
        --domain hotmail.com:8.8 \
        --domain icloud.com:1.2 \
        --domain live.com:0.3 \
        --domain me.com:0.2 \
        --domain msn.com:0.5 \
        --domain orange.fr:0.4 \
        --domain outlook.com:1.2 \
        --domain sbcglobal.net:0.3 \
        --domain yahoo.com:5.9 \
        $@
```

The preceding example uses the domain argument to list the destination domains that should be generated and their relative weights.

When performing raw throughput testing, it can be helpful to use [custom
routing](https://docs.kumomta.com/userguide/policy/routing/) to configure the
test server to route all messages to the sink server, with the sink configured
to dev/null all messages. Modify the `init.lua` on the test server with the
following:

```lua
kumo.on('smtp_server_message_received', function(msg)
  msg:set_meta('queue', 'my.sink.server')
end)
```

However, when using the above smart hosting/routing technique you must be aware
that it causes the outgoing traffic to fan in to a smaller-than-real-world set
of egress sources.  It is suitable for measuring the maximum throughput
possible, but will not reflect the system behavior in terms of managing queues
and respecting your shaping settings for the various destination sites.

For a truer representation of the overall system behavior we recommend using
your firewall to redirect traffic to the sink in a transparent manner.  You
can use the `iptables` command for this purpose:

```console
$ iptables -t nat -A OUTPUT -p tcp \! -d 192.168.1.0/24 \
  --dport 25 -j DNAT --to-destination 127.0.0.1:2026
```

In the preceding example all traffic, other than LAN traffic on 192.168.1.0/24,
destined for port 25 is instead routed to localhost on port 2026.

!!! note
    You will need to disable
    [MTA-STS](../../reference/kumo/make_egress_path/enable_mta_sts.md) and
    [DANE](../../reference/kumo/make_egress_path/enable_dane.md) when using
    this sort of redirection, otherwise you will experience TLS failures for
    sites that publish MTA-STS and/or DANE policies.

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
