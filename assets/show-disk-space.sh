#!/bin/sh
echo "Memory and Swap"
free
echo

echo "Disk Space"
df -h
echo

if test -d target ; then
  echo "Target dir (total)"
  du -sh target
  echo
fi

if test -d target/debug ; then
  echo "Target dir (debug)"
  du -sh target/debug/*
  echo
fi

if test -d target/release ; then
  echo "Target dir (release)"
  du -sh target/release/*
  echo
fi
