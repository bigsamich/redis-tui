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
$CLI SET "string:url" "https://example.com/api/v2/data?format=json&limit=500" >/dev/null
$CLI SET "string:multiline" "line one\nline two\nline three\nline four" >/dev/null

# String with TTL
$CLI SET "string:ephemeral" "I expire in 300 seconds" >/dev/null
$CLI EXPIRE "string:ephemeral" 300 >/dev/null

# Binary blob: 16 float32 values (little-endian sine wave)
# Values: sin(0), sin(pi/8), sin(pi/4), ... sin(15*pi/8)
python3 -c "
import struct, math
vals = [math.sin(i * math.pi / 8) for i in range(16)]
blob = struct.pack('<16f', *vals)
import sys; sys.stdout.buffer.write(blob)
" | $CLI -x SET "blob:float32_sine" >/dev/null

# Binary blob: 32 uint16 values (little-endian ramp)
python3 -c "
import struct
vals = list(range(0, 3200, 100))
blob = struct.pack('<32H', *vals)
import sys; sys.stdout.buffer.write(blob)
" | $CLI -x SET "blob:uint16_ramp" >/dev/null

# Binary blob: 64 int8 values (square wave)
python3 -c "
import struct
vals = [100 if (i // 8) % 2 == 0 else -100 for i in range(64)]
blob = struct.pack('<64b', *vals)
import sys; sys.stdout.buffer.write(blob)
" | $CLI -x SET "blob:int8_square" >/dev/null

# Binary blob: 8 float64 values (exponential)
python3 -c "
import struct, math
vals = [math.exp(i * 0.5) for i in range(8)]
blob = struct.pack('<8d', *vals)
import sys; sys.stdout.buffer.write(blob)
" | $CLI -x SET "blob:float64_exp" >/dev/null

# Binary blob: mixed noise (raw bytes for hex view)
python3 -c "
import os, sys
sys.stdout.buffer.write(os.urandom(128))
" | $CLI -x SET "blob:random_128b" >/dev/null

# ─── Hashes ───────────────────────────────────────────────
$CLI HSET "hash:user:1001" name "Alice" email "alice@example.com" age 30 role "admin" active "true" >/dev/null
$CLI HSET "hash:user:1002" name "Bob" email "bob@example.com" age 25 role "user" active "true" >/dev/null
$CLI HSET "hash:server:config" host "0.0.0.0" port "8080" workers "4" timeout "30" tls "enabled" >/dev/null
$CLI HSET "hash:metrics" cpu_pct "23.5" mem_mb "512" disk_gb "47.2" uptime_hrs "168" requests "984321" >/dev/null

# ─── Lists ─────────────────────────────────────────────────
$CLI RPUSH "list:task_queue" "send_email:user@test.com" "resize_image:photo_001.jpg" "generate_report:Q4_2025" "sync_inventory:warehouse_3" "notify:slack:#ops" >/dev/null
$CLI RPUSH "list:recent_errors" \
    '{"ts":"2025-12-01T10:00:00Z","msg":"connection timeout","code":504}' \
    '{"ts":"2025-12-01T10:05:12Z","msg":"disk full","code":500}' \
    '{"ts":"2025-12-01T10:11:33Z","msg":"auth failed","code":401}' \
    '{"ts":"2025-12-01T10:20:00Z","msg":"rate limited","code":429}' >/dev/null
$CLI RPUSH "list:numbers" 10 20 30 40 50 60 70 80 90 100 >/dev/null

# ─── Sets ──────────────────────────────────────────────────
$CLI SADD "set:active_sessions" "sess_a1b2c3" "sess_d4e5f6" "sess_g7h8i9" "sess_j0k1l2" "sess_m3n4o5" >/dev/null
$CLI SADD "set:tags" "rust" "redis" "tui" "ratatui" "cli" "database" "visualization" >/dev/null
$CLI SADD "set:blocked_ips" "192.168.1.100" "10.0.0.55" "172.16.0.99" >/dev/null

# ─── Sorted Sets ──────────────────────────────────────────
$CLI ZADD "zset:leaderboard" 9500 "alice" 8700 "bob" 8200 "charlie" 7100 "diana" 6500 "eve" 5900 "frank" 4200 "grace" 3100 "heidi" >/dev/null
$CLI ZADD "zset:api_latency_ms" 12.5 "/health" 45.2 "/api/users" 120.8 "/api/search" 230.1 "/api/export" 5.1 "/api/ping" 89.3 "/api/upload" >/dev/null
$CLI ZADD "zset:temperatures" -10.5 "jan" -2.3 "feb" 5.0 "mar" 12.8 "apr" 20.1 "may" 26.5 "jun" 30.2 "jul" 29.0 "aug" 22.4 "sep" 14.1 "oct" 5.5 "nov" -5.2 "dec" >/dev/null

# ─── Streams ──────────────────────────────────────────────

# Stream with text fields (like a log)
$CLI XADD "stream:app_log" "*" level INFO msg "Application started" service "api" >/dev/null
$CLI XADD "stream:app_log" "*" level WARN msg "High memory usage detected" service "worker" >/dev/null
$CLI XADD "stream:app_log" "*" level ERROR msg "Database connection lost" service "api" >/dev/null
$CLI XADD "stream:app_log" "*" level INFO msg "Reconnected to database" service "api" >/dev/null
$CLI XADD "stream:app_log" "*" level DEBUG msg "Cache hit ratio: 0.94" service "cache" >/dev/null

# Stream with binary _data blobs (sensor-style, little-endian float32)
# Each entry has a text field and a _ blob field
for i in $(seq 0 19); do
    BLOB=$(python3 -c "
import struct, math, sys
t = $i * 0.5
temp = 20.0 + 5.0 * math.sin(t) + 0.5 * (($i * 7) % 3 - 1)
humidity = 60.0 + 10.0 * math.cos(t * 0.7)
pressure = 1013.25 + 2.0 * math.sin(t * 0.3)
accel_x = 0.01 * math.sin(t * 2.0)
accel_y = 0.01 * math.cos(t * 2.0)
accel_z = 9.81 + 0.005 * math.sin(t * 5.0)
blob = struct.pack('<6f', temp, humidity, pressure, accel_x, accel_y, accel_z)
sys.stdout.buffer.write(blob)
" | base64)
    # Use raw bytes via pipeline
    python3 -c "
import struct, math, sys
t = $i * 0.5
temp = 20.0 + 5.0 * math.sin(t) + 0.5 * (($i * 7) % 3 - 1)
humidity = 60.0 + 10.0 * math.cos(t * 0.7)
pressure = 1013.25 + 2.0 * math.sin(t * 0.3)
accel_x = 0.01 * math.sin(t * 2.0)
accel_y = 0.01 * math.cos(t * 2.0)
accel_z = 9.81 + 0.005 * math.sin(t * 5.0)
blob = struct.pack('<6f', temp, humidity, pressure, accel_x, accel_y, accel_z)
# Build RESP protocol for XADD
args = ['XADD', 'stream:sensor_data', '*', 'sensor_id', 'env-001', '_data', blob.hex()]
sys.stdout.write('Sensor entry $i done\n')
" >/dev/null
    # Actually send via redis-cli using the python blob
    python3 -c "
import struct, math, sys
t = $i * 0.5
temp = 20.0 + 5.0 * math.sin(t) + 0.5 * (($i * 7) % 3 - 1)
humidity = 60.0 + 10.0 * math.cos(t * 0.7)
pressure = 1013.25 + 2.0 * math.sin(t * 0.3)
accel_x = 0.01 * math.sin(t * 2.0)
accel_y = 0.01 * math.cos(t * 2.0)
accel_z = 9.81 + 0.005 * math.sin(t * 5.0)
blob = struct.pack('<6f', temp, humidity, pressure, accel_x, accel_y, accel_z)
sys.stdout.buffer.write(blob)
" | $CLI -x XADD "stream:sensor_data" "*" sensor_id "env-001" _data >/dev/null
done

# Stream with uint16 blob data (simulated ADC readings)
for i in $(seq 0 29); do
    python3 -c "
import struct, math, sys
t = $i
ch0 = int(2048 + 2000 * math.sin(t * 0.3))
ch1 = int(2048 + 1500 * math.cos(t * 0.2))
ch2 = int(1000 + 500 * (t % 5))
ch3 = int(4000 - 100 * (t % 10))
blob = struct.pack('<4H', ch0, ch1, ch2, ch3)
sys.stdout.buffer.write(blob)
" | $CLI -x XADD "stream:adc_readings" "*" device "adc-042" _samples >/dev/null
done

# ─── Some keys in DB 1 ────────────────────────────────────
$CLI -n 1 SET "db1:test_key" "This is in database 1" >/dev/null
$CLI -n 1 HSET "db1:info" description "Alternate database" purpose "testing" >/dev/null

# ─── Summary ──────────────────────────────────────────────
echo ""
echo "=== Test Data Loaded ==="
echo ""
echo "DB 0:"
echo "  Strings:      5 plain + 1 with TTL + 5 binary blobs"
echo "  Hashes:       4 (users, config, metrics)"
echo "  Lists:        3 (task queue, errors, numbers)"
echo "  Sets:         3 (sessions, tags, IPs)"
echo "  Sorted Sets:  3 (leaderboard, latency, temperatures)"
echo "  Streams:      3 (app_log, sensor_data with _data blobs, adc_readings with _samples blobs)"
echo ""
echo "DB 1:"
echo "  2 keys (string + hash)"
echo ""
echo "Total keys in DB 0: $($CLI DBSIZE | tr -d '\r')"
echo ""
echo "Binary blob guide:"
echo "  blob:float32_sine   - 16x float32 LE  (sine wave)"
echo "  blob:uint16_ramp    - 32x uint16 LE   (linear ramp)"
echo "  blob:int8_square    - 64x int8         (square wave)"
echo "  blob:float64_exp    - 8x float64 LE    (exponential)"
echo "  blob:random_128b    - 128 random bytes (hex view)"
echo ""
echo "Stream blob guide:"
echo "  stream:sensor_data  _data field    = 6x float32 LE (temp, humidity, pressure, accel xyz)"
echo "  stream:adc_readings _samples field = 4x uint16 LE  (4 ADC channels)"
echo ""
echo "=== Starting Redis TUI ==="
echo ""

cargo run
