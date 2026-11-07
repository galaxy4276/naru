# Builds naru-d, starts it with HOME=$1 and sets daemon_pid and daemon_ws.
start_daemon() {
  local home=$1 state="$1/Library/Application Support/Naru"
  mkdir -p "$home"
  cargo build -q --manifest-path "$root/Cargo.toml" -p naru-d
  rm -f "$state/daemon.json"
  # Brain's note lives in the vault, outside HOME; a throwaway daemon gets a note of its own.
  mkdir -p "$home/vault"
  HOME=$home NARU_BRAIN_NOTE="$home/vault/Brain.md" "$root/target/debug/naru-d" >>"$home/naru-d.log" 2>&1 &
  daemon_pid=$!
  local tries=0
  until [[ -f $state/daemon.json ]]; do
    (( ++tries > 100 )) && { print -u2 "naru-d did not start; see $home/naru-d.log"; return 1; }
    sleep 0.1
  done
  local port=$(/usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["port"])' "$state/daemon.json")
  daemon_ws="ws://127.0.0.1:$port/ws?token=$(<"$state/ui-token")"
}
