# The History of KumoMTA

KumoMTA is an open source Message Transfer Agent (MTA) designed to provide high performance outbound email functionality.

The KumoMTA project was founded by a group of email industry veterans with decades of experience building and managing high-performance On-Prem MTAs and is supported by a community of some of the largest senders in the world. While paying attention to the lessons of history, KumoMTA was designed from the ground up with new tech as opposed to modifying something that already existed.  We specifically avoided making a modification of Postfix or Exim or some other existing MTA and instead wrote entirely new code in [Rust](https://www.rust-lang.org/).

## What is a "Kumo"?
So how did we come up with the name **KumoMTA**?  We set out to build a cloud deployable on-premises MTA that was flexible enough to install both on bare metal and in a public or private cloud.  Kumo means cloud in Japanese, so "**KumoMTA**" is a Cloud MTA.

## Why Open Source?
High volume commercial MTAs tend to have closed source code and steep license fees. Neither of these are particularly bad as long as the software is maintained and is flexible enough to modify.  However, the kind of people who typically install very complex high-volume MTAs usually want to modify it or integrate it into other systems.  Providing an open source option allows people to modify the code as needed, and contribute modifications easily to the community.  The email community is full of very smart, creative people who now have an avenue to contribute to a wider community project.
