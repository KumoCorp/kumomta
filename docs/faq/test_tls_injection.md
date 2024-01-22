# How Can I Test TLS Injection?

While it is straightforward to test SMTP message injection against KumoMTA using telnet to port 25 on the KumoMTA instance, you cannot test a session involving TLS (or SMTP AUTH, which requires TLS), using telnet due to the encrypted nature of a TLS connection.

Instead, we recommend using Swaks for all testing, not only for TLS and AUTH but for the sake of repeatability, especially when [asking for help](../userguide/general/get_help.md).

Swaks, or [Swiss Army Knife for SMTP](https://www.jetmore.org/john/code/swaks/), is a flexible and effective testing tool for all manners of MTAs and SMTP scenarios.

## Installing Swaks

Installing Swaks can be as simple as pulling a copy using Curl and setting permissions:

```bash
curl -O https://jetmore.org/john/code/swaks/files/swaks-20240103.0/swaks
chmod 755 ./swaks
```

## Testing TLS and SMTP AUTH with Swaks

The following code shows how to perform a test injection to your local server over tls and authorized with AUTH PLAIN using Swaks:

```bash
[root@localhost ~]# ./swaks --to testrecipient@testdomain.fake --from testsender@testdomain.fake --server 127.0.0.1 --port 587 --auth plain --tls --auth-user someuser --auth-password somepassword
=== Trying 127.0.0.1:587...
=== Connected to 127.0.0.1.
<-  220 testdomain.fake KumoMTA Corp Mailer
 -> EHLO localhost
<-  250-testdomain.fake Aloha localhost
<-  250-PIPELINING
<-  250-ENHANCEDSTATUSCODES
<-  250 STARTTLS
 -> STARTTLS
<-  220 Ready to Start TLS
=== TLS started with cipher TLSv1.3:TLS_AES_256_GCM_SHA384:256
=== TLS client certificate not requested and not sent
=== TLS no client certificate set
=== TLS peer[0]   subject=[/CN=testdomain.fake]
===               commonName=[testdomain.fake], subjectAltName=[DNS:testdomain.fake] notAfter=[2024-02-25T23:19:10Z]
=== TLS peer[1]   subject=[/C=US/O=Let's Encrypt/CN=R3]
===               commonName=[R3], subjectAltName=[] notAfter=[2025-09-15T16:00:00Z]
=== TLS peer[2]   subject=[/C=US/O=Internet Security Research Group/CN=ISRG Root X1]
===               commonName=[ISRG Root X1], subjectAltName=[] notAfter=[2024-09-30T18:14:03Z]
=== TLS peer certificate passed CA verification, failed host verification (using host demo2.kumomta.com to verify)
 ~> EHLO localhost
<~  250-testdomain.fake Aloha localhost
<~  250-PIPELINING
<~  250-ENHANCEDSTATUSCODES
<~  250 AUTH PLAIN
 ~> AUTH PLAIN AHNvbWV1c2VyAHNvbWVwYXNzd29yZA==
<~* 535 5.7.8 AUTH invalid
*** No authentication type succeeded
 ~> QUIT
<~  221 So long, and thanks for all the fish!
=== Connection closed with remote host.
```
