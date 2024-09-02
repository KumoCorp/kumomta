# connect_timeout

How long to wait between starting an SMTP connection and receiving a 220 from a
receiving host. The default is `60s`.

{{since('2024.09.02-c5476b89', inline=True)}}
    The `connect_timeout` is now purely focused on the time it takes to
    establish a working connection. The time allowed for receiving the
    initial 220 banner has been separated out into `banner_timeout`.


