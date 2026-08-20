#!/bin/bash

# Check if last_arch_analysis timestamp file exists, create it if not
TIMESTAMP_FILE="/var/run/last_arch_analysis"
if [ ! -f "$TIMESTAMP_FILE" ]; then
    touch "$TIMESTAMP_FILE"
fi

# Get the current time and the last run time
CURRENT_TIME=$(date +%s)
LAST_RUN_TIME=$(cat "$TIMESTAMP_FILE")

# Calculate the elapsed time since the last run
ELAPSED_TIME=$((CURRENT_TIME - LAST_RUN_TIME))

# Check if 2 hours (7200 seconds) have passed since the last run
if [ "$ELAPSED_TIME" -ge 7200 ]; then
    # Enqueue hex analyze command here
    echo "Hex analysis enqueued"

    # Update the timestamp file with the current time
    echo "$CURRENT_TIME" > "$TIMESTAMP_FILE"
fi