# How it works


This section is for those who want to dig in a litle more on what the internal processes look like. This page will grow as we hear more questions on how the code functions and the answers do not really belong in any other section of the docs.

## What is the actual message flow?

If we imaging watching a message flow through KumoMTA in "[Bullet-Time](https://en.wikipedia.org/wiki/Bullet_time)", the flow looks like this:

* A message to a recipient hits the esmtp or http listener, which only passes it if it complies with the access rules in the listener.
* The message is passed to `smtp_server_message_received`, where you can modify the message and its meta data
* _After_ it returns, we resolve the scheduled queue name by:
 *  Reading msg:get_meta('queue') and using that if it is set
 *  Reading the recipient.domain, msg:get_meta('tenant'), msg:get_meta('campaign') and computing the queue name
* If we haven't already created that scheduled queue, trigger get_queue_config to configure it
* If/when the message is eligible for delivery, call get_egress_pool + get_egress_source to determine the list of sources, then pick one so that we can determine the path that it will take.
* If we haven't already created a ready queue for that path, get_egress_path_config is called and used to create it


