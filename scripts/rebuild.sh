#!/usr/bin/env bash
# Rebuild all hex binaries and restart services
set -e

echo "⬡ Building all hex binaries (release)..."
cargo build --release -p hex-nexus -p hex-cli -p hex-chat

echo "⬡ Installing binaries to bin/..."
mkdir -p bin
cp -f target/release/hex-nexus bin/hex-nexus
cp -f target/release/hex bin/hex
cp -f target/release/hex-chat bin/hex-chat

echo "⬡ Stopping services..."
target/release/hex nexus stop 2>/dev/null || true
pkill -f "hex-chat web" 2>/dev/null || true
sleep 1

echo "⬡ Starting services..."
target/release/hex nexus start

echo ""
echo "⬡ Status:"
target/release/hex nexus status
