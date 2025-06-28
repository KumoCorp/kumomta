#!/bin/bash

# Generate certificates used for test-client-cert.lua execution

mkdir -p /tmp/kumomta/
cd /tmp/kumomta/

openssl genrsa -out key.pem 2048
openssl req -new -key key.pem -out csr.pem -nodes -subj "/C=US/ST=CA/L=SanJose/O=MyOrg/OU=MyUnit/CN=myclient.example.com"
openssl x509 -req -days 365 -in csr.pem -signkey key.pem -out cert.pem

# For rustls
# You'll get the following error
# rfc5321::tls: invalid client side certificate: invalid peer certificate: Other(OtherError(UnsupportedCertVersion))

cat << 'EOF' > csr_v3.cnf
[req]
distinguished_name = example.com
x509_extensions = v3_req
prompt = no

[example.com]
CN = www.example.v3.com

[v3_req]
keyUsage = nonRepudiation, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth, clientAuth
subjectAltName = @alt_names

[alt_names]
DNS.1 = www.example.com
DNS.2 = example.com
DNS.3 = mail.example.com
IP.1 = 192.168.1.100
EOF

openssl req -x509 -newkey rsa:2048 -nodes -keyout v3.key -out v3.crt -days 3650 -config csr_v3.cnf

# for testing only
chmod 644 v3.key v3.crt *.pem
