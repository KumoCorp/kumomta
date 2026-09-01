# Inbound vs outbound mail

Inbound mail is what your infrastructure receives from the internet (MX, filtering, mailbox delivery); outbound mail is what it sends. The two directions have different software requirements: inbound emphasizes filtering, spam rejection, and mailbox storage, while outbound emphasizes queue management, traffic shaping, and sender reputation. Many MTAs are stronger in one direction than the other.
