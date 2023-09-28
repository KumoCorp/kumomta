# Testing KumoMTA

Now that you have KumoMTA installed, you should test it from the command line of the installed host. This is easy if you installed the basic tools as described earlier.

Note that the default SMTP listener is on port 25, so we have use that in these examples.

## Telnet method for SMTP

Start a telnet session with `telnet localhost 25`, replacing youremail@address.com with your actual email address:

```bash
ehlo moto
mail from:youremail@address.com
rcpt to:youremail@address.com
DATA
from:youremail@address.com
to:youremail@address.com
subject: My First Email

Hey, this is my first email!
.
```

!!!note
    If you have not [specifically requested outbound use of port 25](https://aws.amazon.com/premiumsupport/knowledge-center/ec2-port-25-throttle/) from AWS, then it is very possible the message will not be delivered.

## Curl method for HTTP API

```bash
$ curl -H 'Content-Type: application/json' 'http://127.0.0.1:8000/api/inject/v1' -d '{
    "envelope_sender": "noreply@example.com",
    "content": "Subject: hello\n\nHello there",
    "recipients": [
        {
            "email": "recipient@example.com"
        }
    ]
}'
```

See the [HTTP injection API reference](../reference/http/api_inject_v1.md) for
more information.

## Using Swaks for testing

Swaks, the [Swiss Army Knife for SMTP](http://www.jetmore.org/john/code/swaks/) by John Jetmore is a fantastic tool for testing.

Install Swaks:

```bash
curl -O https://jetmore.org/john/code/swaks/files/swaks-20201014.0.tar.gz
tar -xvzf swaks-20201014.0.tar.gz
chmod 755 ./swaks-20201014.0/swaks
```

Basic Swaks usage:

```bash
swaks --to user@example.com --server 127.0.0.1 --port 25
```

Regardless of testing method used, the next step is to [check the logs](./checking_logs.md).