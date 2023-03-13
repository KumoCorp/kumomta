# Your First Email

Now that you have KumoMTA installed, you should test it from the command line of the installed host.  
This is easy if you installed the basic tools as described in the System Preparation section.  
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

## Curl method for HTTP API

```bash
curl 'http://127.0.0.1:8000/api/inject/v1'`
{
    "envelope_sender": "noreply@example.com",
    "content": "Subject: hello\n\nHello there",
    "recipients": [
        {
            "email": "recipient@example.com",
        }
    ]
}
```

## Using Swaks for testing

Swaks, the [Swiss Army Knife for SMTP](http://www.jetmore.org/john/code/swaks/) by John Jetmore is a fantastic tool for testing.

- Click the link above for more detail on how to use Swaks
- As of this writing, you can pull and install the package with

 ```bash
curl -O https://jetmore.org/john/code/swaks/files/swaks-20201014.0.tar.gz
tar -xvzf swaks-20201014.0.tar.gz
chmod 755 ./swaks-20201014.0/swaks
```

You can test a relay through KumoMTA with this (change user@example.com to your own email address first)

```bash
swaks --to user@example.com --server 127.0.0.1 --port 2025
```


## Checking the logs
...

