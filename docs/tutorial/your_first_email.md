# Your First Email

Before actually sending any email, you should configure DKIM. [Read the guide](https://docs.kumomta.com/userguide/configuration/dkim/) for details, but the short version is below.  Replace the domain and selector with your own, then generate signing keys with:
```console
export DOMAIN=<your_domain>
export SELECTOR=<your_selector>
mkdir -p /opt/kumomta/etc/dkim/$DOMAIN
openssl genrsa -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.key 1024
openssl rsa -in /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.key \
 -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.pub -pubout -outform PEM
```

Now that you have KumoMTA installed, you should test it from the command line of the installed host. This is easy if you installed the basic tools as described earlier.  
Note that the default SMTP listener is on port 2025, so we have use that in these examples.

## Telnet method for SMTP

Start a telnet session with ```telnet localhost 2025```
Then replace youremail@address.com with your actual email address.
Copy the entire thing and paste it into the telnet session in your console.

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

Note that you could easily do this with nc (netcat) in exactly the same way, I just prefer telnet.

Check your mail to make sure it delivered.

Note that if you have not [specifically requested outbound use of port 25](https://aws.amazon.com/premiumsupport/knowledge-center/ec2-port-25-throttle/) from AWS, then it is very possible the message will not be delivered. 


## Curl method for HTTP API

```console
$ curl -H 'Content-Type: application/json' 'http://127.0.0.1:8000/api/inject/v1' -d '{
    "envelope_sender": "noreply@example.com",
    "content": "Subject: hello\n\nHello there",
    "recipients": [
        {
            "email": "recipient@example.com",
        }
    ]
}'
```

See the [HTTP injection API reference](../reference/http/api_inject_v1.md) for
more information about this.

## Using Swaks for testing

Swaks, the [Swiss Army Knife for SMTP](http://www.jetmore.org/john/code/swaks/) by John Jetmore is a fantastic tool for testing.

- Click the link above for more detail on how to use Swaks
- As of this writing, you can pull and install the package with

```console
curl -O https://jetmore.org/john/code/swaks/files/swaks-20201014.0.tar.gz
tar -xvzf swaks-20201014.0.tar.gz
chmod 755 ./swaks-20201014.0/swaks
```

You can test a relay through KumoMTA with this (change user@example.com to your own email address first)

```console
swaks --to user@example.com --server 127.0.0.1 --port 2025
```


