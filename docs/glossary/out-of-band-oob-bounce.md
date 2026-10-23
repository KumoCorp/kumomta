# Out-of-band (OOB) bounce

An out-of-band bounce is a delivery failure reported after the message was accepted, arriving later as a DSN to the return-path address rather than as a rejection during the SMTP session. Receivers that accept mail first and filter it afterward generate them routinely, so bounce processing has to consume both in-session rejections and OOB reports to see the full failure picture.
