#!/bin/zsh
set -eu

action=$1
domain=$2
label=$3
plist=$4
case "$action" in
  start|restart) ;;
  *) print -u2 '用法：macos-service.zsh start|restart domain label plist'; exit 2 ;;
esac
if /bin/launchctl print "$domain/$label" >/dev/null 2>&1; then
  if [[ "$action" == restart ]]; then
    /bin/launchctl kickstart -k "$domain/$label"
  else
    /bin/launchctl kickstart "$domain/$label"
  fi
else
  /bin/launchctl bootstrap "$domain" "$plist"
fi
