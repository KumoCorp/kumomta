# Environmental Considerations

When deciding on server hardware, senders must balance vertical and horizontal scaling based on their preferences. KumoMTA is built to support both vertical and horizontal scaling, with both single-node performance of several million messages per hour, as well as support for clustered installations.

For illustrative purposes, we will consider a sender who is sending an average of 8 million messages per day, during a core window of 8 hours per day, or one million messages per hour peak throughput.

For this example, we will use an average message size of 50 KB, resulting in 1 Million * 50KB = 50,000,000 KB or 50GB total transfer/hour (400GB/day).

Expressed in terms of throughput, 50GB / 3600s = 111 Mbps (0.1Gbps), something handled easily by Gigabit networking.

This means that our build is targeting a burst speed of one million messages per hour, a common use case for larger scale senders. If your needs are lower, you can certainly provision a smaller server than what will be discussed in this section of the tutorial.

## NETWORK

Because this use case involves slightly over 100Mbps, most network environments will be able to handle this traffic without modification. Be sure to verify that your server (or virtual server host) is connected to a network that supports Gigabit or faster connectivity.

## STORAGE VOLUME

The total volume is important for calculating storage capacity.  KumoMTA does not store the full body after delivery, but it will be needed to calculate spool capacity and memory use.  In a worst case scenario, if all of your messages are deferred (temporarily undeliverable) then your delayed queue is going to need to store all messages until delivery resumes. Given that the messages relaying through the server add up to 400GB per day, you will need 400G in storage capacity to handle a full-day outage.

Under normal circumstances, most messages will deliver in realtime, and queue depth will generally be correlated to your sending reputation; the better your sending reputation, the less spool storage you need, so there is one more good reason to maintain a good sending reputation.

You should allocate enough spool storage to accomodate as many days of sending outage as you feel comfortable tolerating, in most cases a day's worth of storage is sufficient, given that you typically don't see full outbound outages that last more than a few hours in well-managed datacenters.

!!!note
    Spool storage is calculated as ((Average_Message_Size_in_bytes + 512) x message volume)

Logs will also consume drive space. The amount of space is highly dependent on your specific configuration, but based on the default server policy, each log line will consume about 250-500 bytes on disk thanks to KumoMTA's use of `zstd` compression, and there may be an average of 2-5 log lines per message.  In this example we are going to assume logging at a rate of 2KB per message in order to include a safety margin.

So we have 2KB * 8 Million messages = **~16.3Gb** (per day of log storage). If you are looking at storing 30 days of logs, this results in roughly 500GB of storage for logs.

You are also going to need about around 100GB for the OS (including keeping OS logs and other /var elements without the immediate risk of disk overflow), resulting in a total storage allocation of **1Tb**. Note that this assumes retention of a full month of MTA logs, a full day of spool delay, and a sizable partition for OS logs. This number can be brought down significantly through log rotation and a more conservative deffered spool estimate.

When installing the OS, the disks should be partitioned to keep the spool and logs separate from the OS. In addition, separating the /var directory (other than the directories within /var that are used for KumoMTA) to its own partition is highly recommended.

## CPU
KumoMTA is built on a multithreaded scheduler architecture and takes full advantage of multiple CPU cores. While it can operate on a single (v)CPU, we recommend starting with at least four CPU cores to allow for workload between things like DKIM signing, connection handling, and IO management to be spread out more efficiently.

## RAM
KumoMTA will process as many messages in RAM as possible, so more is better.  We recommend 16Gb RAM, but you will see benefits from adding more as your message processing volume increases.

So, from all of that we see a need for 4 vCPUs, 16Gb RAM, and 1TB of storage. In AWS, that translates to somewhere between an xlarge to a 4xlarge instance size, depending on instance type.

With our server selected, we can [install the OS and prepare it](./system_preparation.md).