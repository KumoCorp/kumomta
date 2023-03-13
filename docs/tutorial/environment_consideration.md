# KumoMTA Environmental Considerations

## Selecting a server or Instance

KumoMTA is a performance MTA that will leverage every bit of power you provide. It may be kind of obvious, but 'more is better' so if you plan to send many millions of messages per hour, deploy the largest server you can. You can deploy in bare metal, public or private cloud, with or without Kubernetes.

## Operating Systems

So far we've run non-production tests on the following systems:

* Rocky (8,9)
* Alma (8,9)
* OpenSuse Leap (15.4)
* Ubuntu (22)
* AL2
* CentOS7

and the following machine types:

* AWS
* Azure
* GCP
* VMWare
* bare metal

## RAM and Storage

At an absolute minimum, you will need 4Gb RAM and 20Gb Storage.  KumoMTA makes heavy use of both resources and response time is going to be a factor.  For high performance systems you will want to select storage with the fastest IOPS and lowest latency, so local disk is going to be much better than NAS or SAN. Likewise, you can benefit from faster RAM if it is available. In AWS, a t2.medium is adequate for a minimal install.  If you are installing a Docker Image, the same guide applies. See the chart below for sample performance reports.

## Network Interfaces

KumoMTA is capable of processing many millions of message per hour, or more relevant to this conversation, many thousands of bytes per second.  Your network interface could be your biggest bottleneck.  Below is a quick calculation:

Assuming the average message is 50kB and you plan to send 1 Million of those per hour, your bandwidth requirement will be:

```txt
50 * 8000 * 1,000,000 / 3600s =~ 111Mbps
```

You can see that a 10Mbps Network interface would fail you quickly.  Any performance system should use at least a 10Gb NIC.

## Ports and Security

Note that in order for KumoMTA to bind to port 25 for inbound mail, it must be run as a privileged user.

Note also that if you are deploying to any public cloud, outbound port 25 is probably blocked by default. If this node specificially needs to send mail directly on port 25 to the public internet, you should request access to the port from the cloud provider.  Some hints are below.

|Provider|Resource|
|--------|--------|
|AWS     |[EC2 port 25 throttle](https://aws.amazon.com/premiumsupport/knowledge-center/ec2-port-25-throttle/)|
|Azure   |[Troubleshoot Outbound SMTP Connectivity](https://learn.microsoft.com/en-us/azure/virtual-network/troubleshoot-outbound-smtp-connectivity)|
|GCP     |[Sending Mail](https://cloud.google.com/compute/docs/tutorials/sending-mail)|


