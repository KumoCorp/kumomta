# Delivery event logs

Delivery event logs are the structured per-message records an MTA emits (reception, delivery, deferral, bounce, complaint) with timestamps, destination responses, and metadata. They feed analytics, billing, bounce processing, and incident forensics; at millions of messages per day, log pipeline design is a real engineering problem.
