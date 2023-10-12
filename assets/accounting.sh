#!/bin/bash
# This utility summarizes the last 12 months of activity
DB=${1:-"/var/spool/kumomta/accounting.db"}

sqlite3 -box "${DB}" "select event_time, received, delivered, max(received, delivered) as volume from accounting order by event_time desc limit 12"
