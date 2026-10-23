#!/usr/bin/env bash
# This script is used to validate that all appropriate privileges
# are dropped by kumod when using the `--user` option.
# KUMOD_TEST_USER should be set to the user you want it to switch to.
# $1 is the path to the kumod binary to test, defaulting to target/debug/kumod

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "run this script as root" >&2
  exit 1
fi

user=${KUMOD_TEST_USER:-wez}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
kumod_bin=${1:-"$repo_root/target/debug/kumod"}

if [[ ! -x $kumod_bin ]]; then
  echo "kumod binary is not executable: $kumod_bin" >&2
  exit 1
fi

expected_uid=$(id -u "$user")
expected_gid=$(id -g "$user")
expected_groups=$(id -G "$user" | tr ' ' '\n' | sort -nu | paste -sd' ' -)
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
chmod 755 "$tmpdir"

cat >"$tmpdir/policy.lua" <<'LUA'
local kumo = require 'kumo'

local function print_status(label, path)
  local status = assert(io.open(path, 'r'))
  io.stdout:write(label, '_BEGIN\n')
  io.stdout:write(status:read('*a'))
  io.stdout:write(label, '_END\n')
  status:close()
end

kumo.on('main', function()
  print_status('KUMO_MAIN_PROCESS_STATUS', '/proc/self/status')
  print_status('KUMO_MAIN_THREAD_STATUS', '/proc/thread-self/status')
end)

kumo.on('init', function()
  print_status('KUMO_INIT_PROCESS_STATUS', '/proc/self/status')
  print_status('KUMO_INIT_THREAD_STATUS', '/proc/thread-self/status')
end)
LUA
chmod 644 "$tmpdir/policy.lua"

if ! main_output=$(KUMOD_LOG=error "$kumod_bin" \
  --user "$user" \
  --policy "$tmpdir/policy.lua" \
  --script 2>&1); then
  echo "$main_output" >&2
  exit 1
fi

if ! init_output=$(KUMOD_LOG=error "$kumod_bin" \
  --user "$user" \
  --policy "$tmpdir/policy.lua" \
  --validate 2>&1); then
  echo "$init_output" >&2
  exit 1
fi

output="$main_output"$'\n'"$init_output"

extract_status() {
  local label=$1
  awk -v begin="${label}_BEGIN" -v end="${label}_END" '
    $0 == begin { capture = 1; next }
    $0 == end { capture = 0 }
    capture { print }
  ' <<<"$output"
}

field() {
  local status=$1
  awk -v name="$2:" '$1 == name { print $2; exit }' <<<"$status"
}

validate_status() {
  local label=$1
  local status=$2
  local cap_inh cap_prm cap_eff cap_amb groups uid gid
  local -a uids gids

  read -r -a uids <<<"$(awk '$1 == "Uid:" { print $2, $3, $4, $5 }' <<<"$status")"
  read -r -a gids <<<"$(awk '$1 == "Gid:" { print $2, $3, $4, $5 }' <<<"$status")"
  groups=$(awk '$1 == "Groups:" { for (n = 2; n <= NF; n++) print $n }' <<<"$status" |
    sort -nu | paste -sd' ' -)

  if [[ ${#uids[@]} -ne 4 || ${#gids[@]} -ne 4 ]]; then
    echo "FAIL: could not read Uid/Gid from $label status" >&2
    echo "$status" >&2
    exit 1
  fi

  for uid in "${uids[@]}"; do
    if [[ $uid != "$expected_uid" ]]; then
      echo "FAIL: $label Uid is '${uids[*]}', expected '$expected_uid $expected_uid $expected_uid $expected_uid'" >&2
      exit 1
    fi
  done

  for gid in "${gids[@]}"; do
    if [[ $gid != "$expected_gid" ]]; then
      echo "FAIL: $label Gid is '${gids[*]}', expected '$expected_gid $expected_gid $expected_gid $expected_gid'" >&2
      exit 1
    fi
  done

  if [[ $groups != "$expected_groups" ]]; then
    echo "FAIL: $label Groups is '$groups', expected '$expected_groups'" >&2
    exit 1
  fi

  cap_inh=$(field "$status" CapInh)
  cap_prm=$(field "$status" CapPrm)
  cap_eff=$(field "$status" CapEff)
  cap_amb=$(field "$status" CapAmb)

  if [[ $cap_inh != 0000000000000000 ]]; then
    echo "FAIL: $label CapInh is $cap_inh" >&2
    exit 1
  fi

  if [[ $cap_prm != 0000000000000400 ]]; then
    echo "FAIL: $label CapPrm is $cap_prm, expected CAP_NET_BIND_SERVICE only" >&2
    exit 1
  fi

  if [[ $cap_eff != 0000000000000400 ]]; then
    echo "FAIL: $label CapEff is $cap_eff, expected CAP_NET_BIND_SERVICE only" >&2
    exit 1
  fi

  if [[ $cap_amb != 0000000000000000 ]]; then
    echo "FAIL: $label CapAmb is $cap_amb" >&2
    exit 1
  fi

  printf 'PASS: %s Uid=%s Gid=%s CapPrm=%s CapEff=%s\n' \
    "$label" "${uids[*]}" "${gids[*]}" "$cap_prm" "$cap_eff"
}

main_process_status=$(extract_status KUMO_MAIN_PROCESS_STATUS)
main_thread_status=$(extract_status KUMO_MAIN_THREAD_STATUS)
main_process_pid=$(field "$main_process_status" Pid)
main_process_tgid=$(field "$main_process_status" Tgid)
main_thread_pid=$(field "$main_thread_status" Pid)
main_thread_tgid=$(field "$main_thread_status" Tgid)

if [[ -z $main_process_pid || $main_process_pid == "$main_thread_pid" || $main_process_tgid != "$main_thread_tgid" ]]; then
  echo "FAIL: main callback did not report a distinct worker in the kumod process" >&2
  exit 1
fi

validate_status main-process "$main_process_status"
validate_status main-worker-thread "$main_thread_status"
validate_status init-process "$(extract_status KUMO_INIT_PROCESS_STATUS)"
validate_status init-thread "$(extract_status KUMO_INIT_THREAD_STATUS)"
