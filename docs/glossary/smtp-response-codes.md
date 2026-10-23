# SMTP response codes

SMTP response codes are the numeric replies a receiving server gives to each command: 2xx means accepted, 4xx means a temporary failure (try again later), and 5xx means a permanent rejection (don't retry). The 4xx/5xx distinction drives all queue behavior; 4xx responses produce deferrals and retries, 5xx responses produce bounces. Enhanced status codes (RFC 3463, e.g. 5.7.1) and human-readable text add detail that bounce classifiers parse.
