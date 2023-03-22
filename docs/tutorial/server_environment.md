# Picking a Server
Considerations here have to balance performance with cost.  With the requirement of 8 million messages per day, we are also going to assume that is not evenly distributed.  In our scenario, we will assume those eight million messages will be sent roughly evenly over an 8 hour window (how convenient...).

Lets do some math!

8 Million / 8 hours = 1 Million/hour

1 Million * 50KB = 50GB total volume/hour (400GB/day)

50GB / 3600s = 111 Mbps (0.1Gbps) Bandwidth

Knowing this we can plan the server size we need.  Sending 1 Million 50KB Messages per hour is actually pretty typical in the world of high performance MTAs so this sample build should suit a large portion of readers.

### NETWORK
The bandwidth above is important because AWS EC2 network interfaces typically [support 5Gbps](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-network-bandwidth.html) at the low end.  This means you dont need to stress about adding any network capacity for this build.

### STORAGE VOLUME
The total volume is inportant for calculating storage capacity.  KumoMTA does not store the full body after delivery, but it will be needed to calculate spool capacity and memory use.  In a worst case scenario, if all of your messages are deferred (temporarily undeliverable) then your delayed queue is going to need to store ALL them.  In the example case, we may need to store 400GB\* for the full day's queue.

In reality, most of that will drain in realtime and the better your sending reputation, the less spool storage you need, so there is one more good reason to maintain a good sending reputation. For the purposes of this sample, we are going to assume a 50% delivery rate so we will only need half that volume for spool.

\* _Spool storage is calculated as ((Average_Message_Size_in_bytes + 512) x volume)_

Logs will also consume drive space.  The amount of space is highly dependent on your specific configuration, but based on the default sample policy, each log line will consume about 1KB and there may be an avarage of 5 log lines per message.  In this example we are going to assume logging at a rate of 5KB per mesage and then double it for safety. _We can talk more about reducing log space requirements ina later section._

(5KB * 8Million) * 2 = 80Gb (per day of log storage)

You are also going to need about 8Gb for OS which brings our total storage space need to 8 + 80 + 200 ~= lets call it 300GB.

### CPU
While you could theoretically get by with 1 vCPU, email is actually pretty hard on the number crunching when you do it right.  Cryptographic signing and rate calculations are only a couple of the factors in planning CPU utilization. We recommend a minimum of 4vCPUs and you will see benefits to increasing that as your message processing volume increases.

### RAM
KumoMTA will process as many messages in RAM as possible, so more is better.  We recommend 16Gb RAM, but you will see benefits from adding more as your message processing volume increases.

So, from all of that we see a need for 4 vCPUs, 16Gb RAM, and 300Gb.   In AWS, that translates to a t2.xlarge.

