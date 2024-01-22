# How Do I Configure POP3/IMAP?

KumoMTA is a [Message Transfer Agent](https://en.wikipedia.org/wiki/Message_transfer_agent), or MTA.

The role of an MTA in your [email infrastructure](https://en.wikipedia.org/wiki/Email_agent_\(infrastructure\)) is to relay messages between systems, typically with an MTA configured for sending messages, and another MTA configured in the recipient's network to receive those messages.

Where an MTA is designed to relay messages, storing them only temporarily for as long as needed to deliver the message to the next host, a [Message Delivery Agent](https://en.wikipedia.org/wiki/Message_delivery_agent), or MDA, is designed to store messages until they are retrieved by the user's email client over POP3 or IMAP.

KumoMTA is not designed nor intended to be used as an MDA, and users will need to look elsewhere for MDA functionality.