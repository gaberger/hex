#!/bin/bash

# Ensure ~/.hex/bin directory exists
mkdir -p ~/.hex/bin/

# Copy hex-nexus binary to ~/.hex/bin/
cp target/release/hex-nexus ~/.hex/bin/hex-nexus

# Restart hex-nexus daemon
hex nexus stop && hex nexus start