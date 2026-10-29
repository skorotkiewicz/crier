#!/usr/bin/env bash
# Exercise TCP locally and MQTT through a real, external broker.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/test-flow.sh [--binary FILE] [--broker HOST] [--port PORT]

Build Crier, test TCP locally, and test MQTT through a real external broker.
Requires Bash, Cargo, and access to the MQTT broker. No local broker is started.

  --binary FILE   Test an existing binary instead of building. Cargo is optional.
  --broker HOST   MQTT broker hostname, default broker.emqx.io.
                  Must allow anonymous connections, publications, subscriptions.
  --port PORT     Plaintext MQTT broker port, default 1883.
  -h, --help      Show this help.

CRIER_FLOW_TIMEOUT sets each operation's deadline in seconds, default 15.
Each run uses a unique MQTT topic. Processes and temporary files are cleaned up.
EOF
}

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

pass() {
    printf 'PASS: %s\n' "$*"
}

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
binary=''
broker=broker.emqx.io
port=1883
operation_timeout=${CRIER_FLOW_TIMEOUT:-15}

while (($#)); do
    case $1 in
        --binary|--broker|--port)
            (($# >= 2)) || fail "Missing value for $1"
            [[ -n $2 ]] || fail "Empty value for $1"
            case $1 in
                --binary) binary=$2 ;;
                --broker) broker=$2 ;;
                --port) port=$2 ;;
            esac
            shift 2
            ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "Unknown argument: $1" ;;
    esac
done

[[ $operation_timeout =~ ^[1-9][0-9]{0,2}$ ]] || fail 'CRIER_FLOW_TIMEOUT must be 1 to 999'
[[ $port =~ ^[0-9]{1,5}$ ]] || fail 'Port must be 1 to 65535'
port=$((10#$port))
((port >= 1 && port <= 65535)) || fail 'Port must be 1 to 65535'

if [[ -z $binary ]]; then
    command -v cargo >/dev/null || fail 'Install Cargo or use --binary FILE'
    cargo build --locked --manifest-path "$project_dir/Cargo.toml" --target-dir "$project_dir/target"
    binary="$project_dir/target/debug/crier"
else
    [[ -f $binary && -x $binary ]] || fail "Binary is not executable: $binary"
    binary=$(cd -- "$(dirname -- "$binary")" && printf '%s/%s' "$PWD" "$(basename -- "$binary")")
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/crier-flow.XXXXXX")
work_dir=$(cd -- "$work_dir" && pwd)
tcp_pid=''
mqtt_pid=''
sender_pid=''

stop_process() {
    local pid=$1 attempt
    [[ -n $pid ]] || return 0
    kill "$pid" 2>/dev/null || true
    for ((attempt = 0; attempt < 40; attempt++)); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.05
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
    fi
    wait "$pid" 2>/dev/null || true
}

cleanup() {
    local status=$? log
    trap - EXIT INT TERM
    set +e
    stop_process "$sender_pid"
    stop_process "$tcp_pid"
    stop_process "$mqtt_pid"
    if ((status != 0)); then
        for log in "$work_dir"/*.log; do
            [[ -f $log ]] || continue
            printf '\n--- %s ---\n' "${log##*/}" >&2
            cat "$log" >&2
        done
    fi
    rm -rf -- "$work_dir"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

wait_for_log() {
    local text=$1 log=$2 pid=$3
    local deadline=$((SECONDS + operation_timeout))
    while ((SECONDS < deadline)); do
        if [[ -f $log ]] && grep -Fq -- "$text" "$log"; then
            return 0
        fi
        kill -0 "$pid" 2>/dev/null || return 1
        sleep 0.05
    done
    return 1
}

wait_for_message() {
    local pid=$1 expected=$2 received=$3
    local deadline=$((SECONDS + operation_timeout))
    while ((SECONDS < deadline)); do
        if cmp -s "$expected" "$received"; then
            return 0
        fi
        kill -0 "$pid" 2>/dev/null || return 1
        sleep 0.05
    done
    return 1
}

# Bound the entire sender process as well as Crier's own network operations.
run_sender() {
    local status=0 deadline=$((SECONDS + operation_timeout))
    "$binary" --config "$work_dir/crier.yml" "$@" >"$work_dir/sender.log" 2>&1 &
    sender_pid=$!
    while kill -0 "$sender_pid" 2>/dev/null; do
        ((SECONDS < deadline)) || fail 'Sender exceeded its deadline'
        sleep 0.05
    done
    wait "$sender_pid" || status=$?
    sender_pid=''
    return "$status"
}

# Single-quote paths for the command template parser, including embedded quotes.
quote_argument() {
    local value=$1
    value=${value//\'/\'\\\'\'}
    printf "'%s'" "$value"
}

printf '{}\n' >"$work_dir/crier.yml"
cat >"$work_dir/handler.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ $# == 1 && $1 == "$CRIER_MESSAGE" ]] || exit 8
[[ $1 != '__crier_flow_fail__' ]] || exit 7
printf '%s' "$1" >"$CRIER_FLOW_OUTPUT.tmp"
mv -- "$CRIER_FLOW_OUTPUT.tmp" "$CRIER_FLOW_OUTPUT"
EOF
handler_command="bash $(quote_argument "$work_dir/handler.sh") '{}'"
token="flow-$RANDOM-$RANDOM"
topic="crier/flow/$$-$RANDOM-$RANDOM-$RANDOM"
cd -- "$work_dir"

printf 'Testing TCP listener and sender...\n'
CRIER_FLOW_OUTPUT="$work_dir/tcp.received" "$binary" --config "$work_dir/crier.yml" \
    listen 127.0.0.1:0 --auth "$token" -m "$handler_command" >"$work_dir/tcp.log" 2>&1 &
tcp_pid=$!
wait_for_log 'Listening on ' "$work_dir/tcp.log" "$tcp_pid" || fail 'TCP listener did not start'
IFS= read -r tcp_line <"$work_dir/tcp.log"
tcp_addr=${tcp_line#Listening on }
[[ $tcp_addr != "$tcp_line" && -n $tcp_addr ]] || fail 'Missing TCP listener address'

message='TCP: quotes "double" '\''single'\'' $(touch crier-flow-injected) `touch crier-flow-injected` ; | & $HOME %PATH%'
printf '%s' "$message" >"$work_dir/tcp.expected"
run_sender send "$tcp_addr" --auth "$token" -m "$message" || fail 'TCP send failed'
wait_for_message "$tcp_pid" "$work_dir/tcp.expected" "$work_dir/tcp.received" || fail 'TCP handler received different text'
[[ ! -e crier-flow-injected ]] || fail 'TCP message executed shell code'
pass 'TCP connection, authentication, literal arguments, handler, and acknowledgement'

if run_sender send "$tcp_addr" --auth wrong-token -m rejected; then
    fail 'TCP accepted the wrong authentication token'
fi
grep -Fq 'ERR:AUTH' "$work_dir/sender.log" || fail 'TCP rejection was not an authentication error'
cmp -s "$work_dir/tcp.expected" "$work_dir/tcp.received" || fail 'Rejected TCP message ran the handler'
pass 'TCP rejects incorrect authentication'

if run_sender send "$tcp_addr" --auth "$token" -m '__crier_flow_fail__'; then
    fail 'TCP reported success for a failed handler'
fi
grep -Fq 'ERR:COMMAND' "$work_dir/sender.log" || fail 'TCP sender did not report the command failure'
pass 'TCP reports handler failures'

if run_sender send "$tcp_addr" --auth "$token" -m $'one\ntwo'; then
    fail 'TCP silently accepted a multiline message'
fi
grep -Fq 'must not contain newlines' "$work_dir/sender.log" || fail 'TCP did not validate message framing'
message='TCP still works after rejected messages'
printf '%s' "$message" >"$work_dir/tcp.expected"
run_sender send "$tcp_addr" --auth "$token" -m "$message" || fail 'TCP listener did not recover'
wait_for_message "$tcp_pid" "$work_dir/tcp.expected" "$work_dir/tcp.received" || fail 'TCP recovery message was not handled'
pass 'TCP rejects multiline frames and continues serving messages'
stop_process "$tcp_pid"
tcp_pid=''

printf 'Testing MQTT listener and sender via %s:%s on %s...\n' "$broker" "$port" "$topic"
CRIER_FLOW_OUTPUT="$work_dir/mqtt.received" "$binary" --config "$work_dir/crier.yml" \
    listen --relay "$broker" --port "$port" -t "$topic" --auth "$token" \
    -m "$handler_command" >"$work_dir/mqtt.log" 2>&1 &
mqtt_pid=$!
wait_for_log 'Listening via ' "$work_dir/mqtt.log" "$mqtt_pid" || fail "MQTT listener did not connect and subscribe to $broker:$port"

message=$'MQTT: first line\nquotes "double" '\''single'\'' $(touch crier-flow-injected) ; & $HOME'
printf '%s' "$message" >"$work_dir/mqtt.expected"
run_sender send --relay "$broker" --port "$port" -t "$topic" --auth "$token" -m "$message" || fail 'MQTT publish failed'
wait_for_message "$mqtt_pid" "$work_dir/mqtt.expected" "$work_dir/mqtt.received" || fail 'MQTT handler received different text'
[[ ! -e crier-flow-injected ]] || fail 'MQTT message executed shell code'
pass 'MQTT connection, subscription, multiline delivery, literal arguments, and broker acknowledgement'

run_sender send --relay "$broker" --port "$port" -t "$topic" --auth wrong-token -m rejected || fail 'Broker rejected the authentication test publication'
wait_for_log 'Message rejected: Auth failed' "$work_dir/mqtt.log" "$mqtt_pid" || fail 'MQTT listener did not reject incorrect authentication'
cmp -s "$work_dir/mqtt.expected" "$work_dir/mqtt.received" || fail 'Rejected MQTT message ran the handler'
pass 'MQTT listener rejects incorrect authentication'

run_sender send --relay "$broker" --port "$port" -t "$topic" --auth "$token" -m '__crier_flow_fail__' || fail 'Broker rejected the handler failure test publication'
wait_for_log 'Message rejected: Command exited with ' "$work_dir/mqtt.log" "$mqtt_pid" || fail 'MQTT listener did not report the handler failure'
cmp -s "$work_dir/mqtt.expected" "$work_dir/mqtt.received" || fail 'Failed MQTT handler replaced the last successful message'
pass 'MQTT listener reports handler failures after broker acknowledgement'

message='MQTT still delivers after rejected messages and failed handlers'
printf '%s' "$message" >"$work_dir/mqtt.expected"
run_sender send --relay "$broker" --port "$port" -t "$topic" --auth "$token" -m "$message" || fail 'Final MQTT publish failed'
wait_for_message "$mqtt_pid" "$work_dir/mqtt.expected" "$work_dir/mqtt.received" || fail 'Final MQTT message did not reach the handler'
pass 'MQTT continues delivering messages'

printf '\nAll TCP and real MQTT flow checks passed.\n'
