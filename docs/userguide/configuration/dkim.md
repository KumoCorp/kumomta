# Configuring DKIM Signing

## What it is

DomainKeys Identified Mail (DKIM) is a mechanism that allows verification of the source and contents of email messages. Using DKIM, sending domains can include a cryptographic signature in outgoing email messages. A message's signature may be verified by an MTA during transit and by the Mail User Agent (MUA) upon delivery. A verified signature indicates the message was sent by the sending domain and the message was not altered in transit. When a DKIM signature fails verification that indicates the message may have been altered during transit or that the sender is fraudulently using the sending domain name. The DKIM specification is located here: [rfc6376](https://datatracker.ietf.org/doc/html/rfc6376).

From the RFC ([section 6.2](https://datatracker.ietf.org/doc/html/rfc6376#section-6.2)):
_"Verifiers wishing to communicate the results of verification to other
   parts of the mail system may do so in whatever manner they see fit.
   For example, implementations might choose to add an email header
   field to the message before passing it on.  Any such header field
   SHOULD be inserted before any existing DKIM-Signature or preexisting
   authentication status header fields in the header field block.  The
   Authentication-Results: header field (RFC5451) MAY be used for this
   purpose."_

This diagram gives a graphical view of how DKIM works.

```mermaid
---
title: DKIM Process flow
---
graph TD
    SMTA["Sending MTA"]
    RMTA["Receiving MTA"]
    MBOX["User Mailbox"]
    DNS
    SMTA -- 2. Send to recipient MTA --> RMTA
    RMTA -- 4. Deliver to mailbox --> MBOX
    SMTA -- 1. Publish Public Key --> DNS
    RMTA -- 3. Get Sender's Public Key --> DNS

   style SMTA fill:orange,color:black
   style RMTA fill:skyblue,color:black
   style DNS fill:#A2E4B8,color:black
   style MBOX fill:#E8DD8E,color:black

```

### Enabling DKIM signing in KumoMTA

A system administrator with access to manage DNS generates a public/private key pair to use for signing all outgoing messages for the domain (multiple key pairs are allowed). The public key is published in DNS, and the private key is made available to their DKIM-enabled outbound email servers. This is step "1" in the diagram.

When an email is sent by an authorized user within the domain, the DKIM-enabled email system uses the stored private key to generate a digital signature of the message. This signature is included in a DKIM-Signature header and prepended to the email. The email is then sent on to the recipient's mail server. This is step "2" in the diagram.


### Generating DKIM Keys

Generate public and private keys for each signing domain and create the DKIM public key DNS records for those domains.

The OpenSSL cryptography toolkit can be used to generate RSA keys for DKIM. As an example, the following openssl commands are used to generate public and private keys for the a domain you choose with a selector you choose. The files can be stored in any directory such as ~/kumomta/keys/, but the default is /opt/kumomta/etc/dkim/.

Replace the domain and selector with your own, then generate signing keys with:
```console
export DOMAIN=<your_domain>
export SELECTOR=<your_selector>
sudo mkdir -p /opt/kumomta/etc/dkim/$DOMAIN
sudo openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.pem -aes-256-cbc
```
At this point you will need to provide a passkey and verify it manually. This may be needed later if you need to examine the certificate. Then continue with:

```console
sudo openssl rsa -in /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.pem -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.key
sudo openssl rsa -in /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.key -pubout -outform PEM -out /opt/kumomta/etc/dkim/$DOMAIN/$SELECTOR.pub
sudo chown kumod:kumod /opt/kumomta/etc/dkim/$DOMAIN -R
```
NOTE: that the above will generate a 2048 bit key pair.

Any DKIM verification implementations must support key sizes of 512, 768, 1024, 1536, and 2048 bits. A signer may choose to sign messages using any of these sizes and may use a different size for different selectors. Larger key sizes provide greater security but impose higher CPU costs during message signing and verification. It is not recommended to use a key size lower than 1024 unless absolutely necessary. Note that Google _requires_ senders to sign with a 1024 bit or greater key size.

The resulting public key should look similar to:

```txt
-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDnkmt7Vty2iLsVCpNCx4+tbufL
xwe+P13AmzYYa9SHIV2Is3G+U4vRlAEg1McK1ssrsjF5GWGSKSeDrYJY06I8ruZS
CpPIHQo85GAkmGbBPHMhZuk8x5XSgI8VkjAZDbiJAwg1U6MV5deWqrzDC8OJ3+RK
KPrbKH5ubT9V9pLKawIDAQAB
-----END PUBLIC KEY-----
```

Once the public and private keys have been generated, create a DNS text record for <SELECTOR>._domainkey.<DOMAIN> (IE: dkim1024._domainkey.example.com). The DNS record contains several DKIM "tag=value" pairs and should be similiar to the record shown below:

default._domainkey.example.com. 86400 IN TXT
"v=DKIM1; k=rsa; h=sha256; t=y; p=MIbBa...DaQAB"

DKIM DNS text record tags are defined below. Do not include the quotes below when including a tag value in the DNS text record.

v= DKIM key record version. The value of this tag must be set to "DKIM1".

k= Key type. This tag defines the syntax and semantics of the p= tag value. Currently, this tag should have the value "rsa".

h= Hash algorithm. Currently, this tag should have the value "sha1" or "sha256".

t= Flags. The only value currently defined is "y". If specified, this tag indicates the signing domain is testing DKIM.

p= DKIM public key value generated as described above.

s= Service Type. If specified, this tag should be set to "*" or "email" which represents all service types or the email service type. Currently, "email" is the only service using this key.

n= Notes. If specified, the value of this tag is quoted-printable text used as a note to anyone reading the DNS text record. The tag is not interpreted by DKIM verification and should be used sparingly because of space limitations of the DNS text record.

## Implement the signing process

Configure KumoMTA to sign emails passing through the MTA with DKIM signatures.  This is done with Lua in policy.  The sample init.lua policy provided with KumoMTA declairs a basic working DKIM signer that you can copy and modify as needed.  This signs a message with RSA256 using a selector "default" on headers 'From', 'To', and 'Subject' using the DKIM key located at example-private-dkim-key.pem. ([More documentation](https://docs.kumomta.com/reference/kumo.dkim/rsa_sha256_signer/))

```lua
local signer = kumo.dkim.rsa_sha256_signer {
  domain = msg:from_header().domain,
  selector = 'default',
  headers = { 'From', 'To', 'Subject' },
  file_name = 'example-private-dkim-key.pem',
}
```

Where you want to enable dkim signing, simply call that signer in policy.

IE:  `msg:dkim_sign(signer)`
