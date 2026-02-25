#!/usr/bin/env bash
set -euo pipefail

REDIS_PORT=6379

echo "=== Redis TUI Dev Environment ==="

# Check for redis-server
if ! command -v redis-server &>/dev/null; then
    echo "ERROR: redis-server not found. Install redis first."
    echo "  Ubuntu/Debian: sudo apt install redis-server"
    echo "  macOS:         brew install redis"
    exit 1
fi

# Check for redis-cli
if ! command -v redis-cli &>/dev/null; then
    echo "ERROR: redis-cli not found."
    exit 1
fi

# Start redis-server in background if not already running
if redis-cli -p "$REDIS_PORT" ping &>/dev/null; then
    echo "[*] Redis already running on port $REDIS_PORT"
else
    echo "[*] Starting redis-server on port $REDIS_PORT..."
    redis-server --port "$REDIS_PORT" --daemonize yes --loglevel warning
    sleep 1
    if ! redis-cli -p "$REDIS_PORT" ping &>/dev/null; then
        echo "ERROR: Failed to start redis-server"
        exit 1
    fi
    echo "[*] Redis started (PID $(redis-cli -p "$REDIS_PORT" INFO server | grep process_id | tr -d '\r' | cut -d: -f2))"
fi

CLI="redis-cli -p $REDIS_PORT"

echo "[*] Flushing existing data..."
$CLI FLUSHALL >/dev/null

echo "[*] Loading test data..."

# ─── Strings ───────────────────────────────────────────────
$CLI SET "string:greeting" "Hello, Redis TUI!" >/dev/null
$CLI SET "string:json_config" '{"debug":true,"log_level":"info","max_connections":100,"features":["auth","caching","streams"]}' >/dev/null
$CLI SET "string:counter" "42" >/dev/null

$CLI SET "string:ephemeral" "I expire in 300 seconds" >/dev/null
$CLI EXPIRE "string:ephemeral" 300 >/dev/null

# ─── Two plain blobs (non-stream) ─────────────────────────
echo "[*] Generating blobs..."

# float32 blob - 1k elements, multi-freq sine
python3 -c "
import struct, math, sys
n = 1000
vals = [math.sin(i*0.01) + 0.5*math.sin(i*0.05) + 0.25*math.sin(i*0.13) for i in range(n)]
sys.stdout.buffer.write(struct.pack(f'<{n}f', *vals))
" | $CLI -x SET "blob:float32_1k" >/dev/null
echo "  blob:float32_1k"

# random bytes blob for hex view
python3 -c "
import os, sys
sys.stdout.buffer.write(os.urandom(256))
" | $CLI -x SET "blob:random_256b" >/dev/null
echo "  blob:random_256b"

# ─── Hashes ───────────────────────────────────────────────
$CLI HSET "hash:user:1001" name "Alice" email "alice@example.com" age 30 role "admin" active "true" >/dev/null
$CLI HSET "hash:server:config" host "0.0.0.0" port "8080" workers "4" timeout "30" tls "enabled" >/dev/null
$CLI HSET "hash:metrics" cpu_pct "23.5" mem_mb "512" disk_gb "47.2" uptime_hrs "168" requests "984321" >/dev/null

# ─── Lists ─────────────────────────────────────────────────
$CLI RPUSH "list:task_queue" "send_email:user@test.com" "resize_image:photo_001.jpg" "generate_report:Q4_2025" >/dev/null
$CLI RPUSH "list:numbers" 10 20 30 40 50 60 70 80 90 100 >/dev/null

# ─── Sets ──────────────────────────────────────────────────
$CLI SADD "set:tags" "rust" "redis" "tui" "ratatui" "cli" "database" "visualization" >/dev/null
$CLI SADD "set:blocked_ips" "192.168.1.100" "10.0.0.55" "172.16.0.99" >/dev/null

# ─── Sorted Sets ──────────────────────────────────────────
$CLI ZADD "zset:leaderboard" 9500 "alice" 8700 "bob" 8200 "charlie" 7100 "diana" 6500 "eve" 5900 "frank" >/dev/null
$CLI ZADD "zset:temperatures" -10.5 "jan" -2.3 "feb" 5.0 "mar" 12.8 "apr" 20.1 "may" 26.5 "jun" 30.2 "jul" 29.0 "aug" 22.4 "sep" 14.1 "oct" 5.5 "nov" -5.2 "dec" >/dev/null

# ─── Streams ──────────────────────────────────────────────

# Text log stream
$CLI XADD "stream:app_log" "*" level INFO msg "Application started" service "api" >/dev/null
$CLI XADD "stream:app_log" "*" level WARN msg "High memory usage detected" service "worker" >/dev/null
$CLI XADD "stream:app_log" "*" level ERROR msg "Database connection lost" service "api" >/dev/null

# Small sensor stream (20 entries, float32 _data)
echo "[*] Generating small sensor stream..."
for i in $(seq 0 19); do
    python3 -c "
import struct, math, sys
t = $i * 0.5
temp = 20.0 + 5.0 * math.sin(t) + 0.5 * (($i * 7) % 3 - 1)
humidity = 60.0 + 10.0 * math.cos(t * 0.7)
pressure = 1013.25 + 2.0 * math.sin(t * 0.3)
accel_x = 0.01 * math.sin(t * 2.0)
accel_y = 0.01 * math.cos(t * 2.0)
accel_z = 9.81 + 0.005 * math.sin(t * 5.0)
sys.stdout.buffer.write(struct.pack('<6f', temp, humidity, pressure, accel_x, accel_y, accel_z))
" | $CLI -x XADD "stream:sensor_data" "*" sensor_id "env-001" _ >/dev/null
done

# ─── Big streams with binary _data ───────────────────────
echo "[*] Generating big streams..."

generate_big_stream() {
    local key=$1
    local count=$2
    local dtype=$3
    local fmt=$4
    local values_per_entry=$5

    echo "  ${key} (${count} entries, ${dtype})..."

    python3 -c "
import struct, math, sys

key = '${key}'
count = ${count}
fmt = '${fmt}'
vpe = ${values_per_entry}
dtype = '${dtype}'

for i in range(count):
    # Each entry is a waveform: multi-freq sine, shifted by entry index
    phase_offset = i * 0.3
    vals = []
    for j in range(vpe):
        t = j / vpe * 2 * math.pi + phase_offset
        v = math.sin(t) + 0.5 * math.sin(3 * t) + 0.25 * math.sin(7 * t)
        if dtype == 'float32' or dtype == 'float64':
            vals.append(v)
        elif dtype == 'int16':
            vals.append(int(max(-32768, min(32767, 16000 * v))))
        elif dtype == 'uint16':
            vals.append(int(max(0, min(65535, 32768 + 18000 * v))))
        elif dtype == 'int32':
            vals.append(int(max(-2**31, min(2**31-1, int(1e8 * v)))))
        elif dtype == 'uint32':
            vals.append(int(max(0, min(2**32-1, 2**31 + int(5e8 * v)))))
        elif dtype == 'int8':
            vals.append(int(max(-128, min(127, 72 * v))))
        elif dtype == 'uint8':
            vals.append(int(max(0, min(255, 128 + 72 * v))))
        else:
            vals.append(v)

    blob = struct.pack(fmt, *vals)

    parts = ['XADD', key, '*', 'source', 'gen', '_']
    resp = f'*{len(parts) + 1}\r\n'
    for p in parts:
        b = p.encode()
        resp += f'\${len(b)}\r\n'
        sys.stdout.buffer.write(resp.encode())
        sys.stdout.buffer.write(b)
        sys.stdout.buffer.write(b'\r\n')
        resp = ''
    sys.stdout.buffer.write(f'\${len(blob)}\r\n'.encode())
    sys.stdout.buffer.write(blob)
    sys.stdout.buffer.write(b'\r\n')
" | $CLI --pipe >/dev/null 2>&1
}

# Args: key, num_entries, dtype, struct_fmt, values_per_entry
# Key names reflect values_per_entry (what gets plotted per entry)
generate_big_stream "stream:float32_500"  100  "float32" "<500f" 500
generate_big_stream "stream:float64_200"  100  "float64" "<200d" 200
generate_big_stream "stream:int16_1000"   100  "int16"   "<1000h" 1000
generate_big_stream "stream:uint16_500"   100  "uint16"  "<500H" 500
generate_big_stream "stream:uint8_2000"   100  "uint8"   "<2000B" 2000
generate_big_stream "stream:int8_1000"    100  "int8"    "<1000b" 1000
generate_big_stream "stream:int32_200"    100  "int32"   "<200i" 200
generate_big_stream "stream:uint32_200"   100  "uint32"  "<200I" 200

# ─── Large streams for FFT stress testing ─────────────────
echo "[*] Generating large streams..."
generate_big_stream "stream:large_f32_10k"  50  "float32" "<10000f" 10000
generate_big_stream "stream:large_i16_20k"  50  "int16"   "<20000h" 20000

# ─── Some keys in DB 1 ────────────────────────────────────
$CLI -n 1 SET "db1:test_key" "This is in database 1" >/dev/null
$CLI -n 1 HSET "db1:info" description "Alternate database" purpose "testing" >/dev/null

# ─── Summary ──────────────────────────────────────────────
echo ""
echo "=== Test Data Loaded ==="
echo "Total keys in DB 0: $($CLI DBSIZE | tr -d '\r')"
echo ""
echo "=== Starting Redis TUI ==="
echo ""

cargo run
