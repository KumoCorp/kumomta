# Envelope sender (Return-Path / bounce address)

The envelope sender, also called the return-path, MAIL FROM, or bounce address, is the address given in the SMTP envelope where delivery failures are reported. Senders typically point it at an address they process automatically (see bounce processing), and SPF authenticates this domain. The receiving server records it in the Return-Path header on delivery.
