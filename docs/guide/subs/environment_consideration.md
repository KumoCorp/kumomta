# KumoMTA Environmental Considerations

## Selecting a server or Instance

KumoMTA is a performance MTA that will leverage every bit of power you provide. It may be kind of obvious, but 'more is better' so if you plan to send many millions of messages per hour, deploy the largest server you can. If you are installing for  development, you will need a minimum of 4Gb RAM, 2 cores and 20Gb Storage. In AWS, a t2.medium is adequate for a minimal install.  If you are installing a Docker Image, the same guide applies. See the chart below for sample performance reports.

## Operating Systems

So far this is tested on Rocky 8, ...

## RAM and Storage 

## Network Interfaces

## Ports and Security
Note that in order for KumoMTA to bind to port 25 for outbound mail, it must be run as a privileged user.
Note also that if you are deploying to any public cloud, outbound port 25 is probably blocked by default. If this node specificially needs to send mail directly on port 25 to the public internet, you should request access to the port from the cloud provider.  Some hints are below.

AWS: https://aws.amazon.com/premiumsupport/knowledge-center/ec2-port-25-throttle/

Azure: https://learn.microsoft.com/en-us/azure/virtual-network/troubleshoot-outbound-smtp-connectivity

GCP: https://cloud.google.com/compute/docs/tutorials/sending-mail



