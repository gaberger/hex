#!/bin/bash

# Existing script content (assumed)
echo "Monitoring system status..."

# New addition to extract failure reason
FAILED_TASKS=$(curl -s http://localhost/memory/failed_tasks)

if [ -n "$FAILED_TASKS" ]; then
    echo "Failed tasks found:"
    for task in $FAILED_TASKS; do
        FAILURE_REASON=$(curl -s http://localhost/memory/task/$task/failure_reason)
        if [ -n "$FAILURE_REASON" ]; then
            echo "Task ID: $task, Failure Reason: ${FAILURE_REASON:0:100}"
        fi
    done
else
    echo "No failed tasks found."
fi