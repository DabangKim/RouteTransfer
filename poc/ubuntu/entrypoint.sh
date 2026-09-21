#!/bin/sh
set -eu
: "${TEST_PASSWORD:?A temporary test password is required}"
printf 'poc:%s\n' "$TEST_PASSWORD" | chpasswd
unset TEST_PASSWORD
ssh-keygen -A >/dev/null
exec /usr/sbin/sshd -D -e
