#!/usr/bin/env python3
import fileinput
from datetime import datetime
from datetime import timedelta
import time
import shutil
import os
import sys
import re

"""
This script accepts kumomta diagnost log files either on stdin
or as a list of filenames.

It will parse each line and re-arrange it to make it easier for
a human to grok it
"""

SYSLOG_PREFIX = re.compile(
    "^(?P<time>\\S+ \\d+ \\d+:\\d+:\\d+) (?P<host>\\S+) (?P<proc>\\S+):\\s+(?P<remainder>.*)"
)
KUMO_PREFIX = re.compile(
    "^(?P<time>\\d+-\\d+-\\d+T\\d+:\\d+:\\d+\\.\\d+Z)\\s+(?P<level>\\S+)\\s+(?P<thread>\\S+)\\s+(?P<remainder>.*)"
)

try:
    # Allow piping through `less -r` and respecting the terminal size
    SIZE = os.get_terminal_size(2)
except:
    SIZE = shutil.get_terminal_size()

B_RED = "\x1b[91m"
B_GREEN = "\x1b[92m"
B_YELLOW = "\x1b[93m"
B_BLUE = "\x1b[94m"
B_MAGENTA = "\x1b[95m"
BOLD_BLACK = "\x1b[1;30m"
HALF_BRIGHT = "\x1b[2m"
NORMAL = "\x1b[0m"

LEVEL_COLORS = {
    "INFO": B_GREEN,
    "WARN": B_YELLOW,
    "ERROR": B_RED,
    "DEBUG": B_BLUE,
    "TRACE": B_MAGENTA,
}


def extract_tagged_item(line):
    """
    The tracing crate puts a bunch of context ahead of the actual log
    message, and in kumomta, that context (populated via `instrument`)
    can be very large.

    This function "parses" these contextual parameters and figures
    out the slice ranges for each of them, as well as the final
    log message.

    It returns the tuple of the original log message, the contextual
    part (string), and the start/end indices of the various parameters
    within that contextual string.
    """
    run = None
    stack = []
    params = []
    depth = 0
    end = 0

    for i, c in enumerate(line):
        if run is None:
            run = i

        if i > 1 and c == " " and depth == 0 and line[i - 1] == ":":
            if len(params) != 0:
                end = i + 1
            break

        if c == "{" or c == "(":
            depth += 1
        elif (c == "}" or c == ")") and depth > 0:
            depth -= 1
            if depth == 0:
                params.append([run, i])
                run = None

    context = line[0:end]
    message = line[end:]
    return (message, context, params)


FIRST_TS = None
LAST_TS = None


def process(line):
    global FIRST_TS
    global LAST_TS

    m = SYSLOG_PREFIX.match(line)
    if m:
        line = m.group("remainder")

    m = KUMO_PREFIX.match(line)
    if not m:
        print(line)
        return

    timestamp = m.group("time")
    t = datetime.strptime(timestamp, "%Y-%m-%dT%H:%M:%S.%fZ")

    if not FIRST_TS:
        FIRST_TS = t
    if not LAST_TS:
        LAST_TS = t
    elapsed_start = t - FIRST_TS
    elapsed_prior = t - LAST_TS
    LAST_TS = t

    # print(elapsed_start, elapsed_prior)

    level = m.group("level")
    # if level == 'TRACE':
    #    return

    thread = m.group("thread")
    message, context, params = extract_tagged_item(m.group("remainder"))

    message = message[0 : SIZE.columns * 2]

    level_width = 5
    thread_width = 16
    width = len(timestamp) + level_width + thread_width + len(message) + 4
    avail = SIZE.columns - width
    if avail < 0:
        avail = SIZE.columns + avail

    excerpt = context[0:avail]

    level_color = LEVEL_COLORS[level] or ""

    print(
        f"{HALF_BRIGHT}{timestamp}{NORMAL} {level_color}{level:5}{NORMAL} {HALF_BRIGHT}{thread:16}{NORMAL} {message} {BOLD_BLACK}{excerpt}{NORMAL}"
    )


for line in fileinput.input(encoding="utf-8"):
    process(line)
