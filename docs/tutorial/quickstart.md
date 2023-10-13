# Quickstart Tutorial

This adbridged set of instructions assumes you are an experienced MailOps administrator looking for the basic commands needed for an install. More detailed instructions are in the [next section of the tutorial](./server_environment.md). This tutorial is not intended to be a replacement for reading the [full documentation](../index.md), but rather will show how to install and configure KumoMTA for a specific environment and serve as a basis to your own unique deployment.

<iframe width="560" height="315" src="https://www.youtube.com/embed/ClJX5mIxy7g?si=GcpBpegzsTRz01H5" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

This tutorial assumes the reader has a basic understanding of Linux administration, experience installing and managing an [MTA](https://en.wikipedia.org/wiki/Message_transfer_agent), a provisioned physical or virtual machine, and a minimal installer for Rocky Linux 9.

1. Provision an AWS t2.xlarge (or larger) instance (or any physical or virtual server with at least 4 CPUs, 16Gb RAM, 300Gb Hard Drive).

    !!!Note
        The hardware here is for a high-throughput sending environment, but KumoMTA can run on a smaller footprint for low-volume environments. if your sending needs are smaller, you can deploy to a server with 1 CPU, 2GB RAM, and 10Gb of storage.

1. [Install Rocky Linux 9](https://docs.rockylinux.org/guides/installation/). A minimal install is sufficient.

1. Update the OS and disable Postfix if needed

    ```bash
    sudo dnf clean all
    sudo dnf update -y
    sudo systemctl stop postfix.service
    sudo systemctl disable postfix.service
    ```

1. Add the KumoMTA repo to your config manager and yum install it like this:

    ```bash
    sudo dnf -y install dnf-plugins-core
    sudo dnf config-manager \
        --add-repo \
        https://openrepo.kumomta.com/files/kumomta-rocky.repo
    sudo yum install kumomta
    ```

    !!!note
        Alternatively you can install the kumomta-dev package in order to take advantage of the latest pre-release features. This is only recommended for testing environments.

1. The instructions above will place a default configuration file at /opt/kumomta/etc/policy/init.lua and start the KumoMTA service, if the service does not start by default it can be started and enabled with the following commands:

    ```bash
    sudo systemctl start kumomta
    sudo systemctl enable kumomta
    ```

    Alternately you can run it manually with :
    ```bash
    sudo /opt/kumomta/sbin/kumod --policy \
      /opt/kumomta/etc/policy/init.lua --user kumod&
    ```

    KumoMTA will now be installed and running the init.lua configuration from `/opt/kumomta/sbin/kumod`.  If you started it manually, the `&` pushes the running process to the background, type 'fg' to bring it forward again.

1. Test your KumoMTA configuration using telnet or the tool of your choice:

    ```bash
    telnet localhost 25
    Trying ::1...
    telnet: connect to address ::1: Connection refused
    Trying 127.0.0.1...
    Connected to localhost.
    Escape character is '^]'.
    220 localhost.localdomain KumoMTA
    ehlo moto
    250-localhost.localdomain Aloha moto
    250-PIPELINING
    250-ENHANCEDSTATUSCODES
    250 STARTTLS
    MAIL FROM:test@example.com
    250 OK EnvelopeAddress("test@example.com")
    RCPT TO:test@example.com
    250 OK EnvelopeAddress("test@example.com")
    DATA
    354 Send body; end with CRLF.CRLF
    Subject: Test Message Using KumoMTA

    This is a test.
    .
    250 OK ids=d7ef132b5d7711eea8c8000c29c33806
    quit
    221 So long, and thanks for all the fish!
    ```

1. View the log entries related to your test message:

    ```console
    $ sudo /opt/kumomta/sbin/tailer --tail /var/log/kumomta
    ```

    ```json
    {
      "type": "Reception",
      "id": "d7ef132b5d7711eea8c8000c29c33806",
      "sender": "test@example.com",
      "recipient": "test@example.com",
      "queue": "example.com",
      "site": "",
      "size": 320,
      "response": {
        "code": 250,
        "enhanced_code": null,
        "content": "",
        "command": null
      },
      "peer_address": {
        "name": "moto",
        "addr": "127.0.0.1"
      },
      "timestamp": 1695847980,
      "created": 1695847980,
      "num_attempts": 0,
      "bounce_classification": "Uncategorized",
      "egress_pool": null,
      "egress_source": null,
      "feedback_report": null,
      "meta": {},
      "headers": {},
      "delivery_protocol": null,
      "reception_protocol": "ESMTP",
      "nodeid": "d8e014c7-eaeb-4683-a56e-61324e91b1fc"
    }
    ```

    !!!note
        In the default configuration, it will take about 10 seconds for the log files to flush and show the log entries in the `tailer` output.
        You can speed that up by changing the `max_segment_duration` in the `init.lua` file, or just by restarting the server via
        `sudo systemctl restart kumomta`.

        These example log entries have been formatted for ease of reading in the documentation.

## Next Steps
This page described a situation where you already have a fully prepared server/instance and just needed basic install instructions. [Read on](./server_environment.md) to look at server selection and sizing, OS preparation, installation, and testing in more detail.
