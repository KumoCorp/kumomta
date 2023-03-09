# Getting Started with KumoMTA


## What is this?

KumoMTA is an open source Message Transfer Agent (MTA) designed to provide high performance outbound email functionality. The project was founded by people with decades of experience building and managing extremely high power On-Prem MTAs and is supported by a community of some of the largest senders in the world. While paying attention to the lessons of history, KumoMTA was designed from the ground up with new tech as opposed to modifying something that already existed.  We specifically avoided making a modification of Postfix or Exim or some other existing MTA and instead wrote entirely new code in [Rust](https://www.rust-lang.org/).

If you have no idea what an MTA is then [this may be a good primer](https://en.wikipedia.org/wiki/Message_transfer_agent) before you get too deep into the documentation here.  If you DO know what an MTA is and you are looking for an open source option to support, then you have found your people.

KumoMTA is deployable as a Docker container if you just want to use it to send mail.  Alternately, you can install as a developer/contributor and have full access to the source code of the core MTA.  Contributions from the community are welcome.

### What is a "Kumo"?
So how did we come up with the name **KumoMTA**?  We set out to build a cloud deployable on-premises MTA that was flexible enough to install on bare metal and in public cloud.  Kumo means cloud in Japanese and we are fans of Japanese culture, so "**KumoMTA**" just kinda made sense at the time :) 

### Why Open Source?
High volume commercial MTAs tend to have closed source code and steep license fees. Neither of these are particularly bad as long as the software is maintained and is flexible enough to modify.  However, the kind of people who typically install very complex high volume MTAs, also usually want to modify it or embed it into other systems.  Providing an open source option allows people to modify the code if needed, and contribute modifications easily to the community.  The email community is full of very smart, creative people who now have an avenue to contribute to a wider community project.

The other reason for open source is accessibility. A user can literally just clone the repo, modify the basic config and be sending email without ever talking to a salesperson or requesting a license. All you need is a little technical skill and a server to deploy on.  Usage is governed by an Apache 2.0 license.

## How do I install it?
That depends.  
 - If you just want to _use_ it to send email, follow the instructions to [**Install For Production Use**](./subs/install_for_production_use.md).
 - If you want to experiment, contrubute, or hack stuff up, follow the instructions to [**Install For Development**](./subs/install_for_development.md).

## What next?
Install the version you need based on your reading above.  Modify your config to make it uniquely yours, then test with a small sample of receivers.
Provide feedback to the project as appropriate and let us know if you want to take the next step to active support and advanced features.

