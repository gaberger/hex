# ADR-260512-structural-smell

## Status: Proposed

## Context

We have identified a structural smell in our system where two parallel task pipelines coexist:
1. `swarm_task ← workplan_executor + hex brain enqueue`
2. `inference_task ← brain-chat @mention`

Currently, nothing polls the `swarm_task` table. Instead, two daemons are used to manage this situation:
- `hex-nexus/src/orchestration/swarm_task_bridge.rs`: Moves bridgeable rows from `swarm_task` into `inference_task` every 30 seconds (batch size of 20).
- `hex-nexus/src/orchestration/swarm_task_drainer.rs`: Flips pending and unassigned tasks older than 24 hours to 'failed' with the comment 'auto-drained: orphaned' every 5 minutes.

The `swarm_task_drainer` daemon refers to this process as a release valve until the workplan executor migrates to using the `inference_task`.

Evidence:
- The `brain-lease swarm 938c205d` shows 12/10604 tasks with ~10,592 drained orphans.
- A surge in task counts from May 12 through May 15 (peak of 1187 on the 14th).

## Decision

To address this structural smell and simplify our architecture, we propose the following actions:

1. **Migrate `workplan_executor` + Hex Brain Enqueue to Write Directly to `inference_task`:**
   - Map necessary fields from `swarm_task` (title, depends_on, payload) onto `inference_task` columns.
   - Update relevant producers in:
     - `hex-cli/src/commands/sched.rs` around lines 4077, 4220, and 4325 (brain-task title emitter).
     - `workplan_executor` (hex-nexus).

2. **One-Shot Garbage Collection of Historical Drained Rows:**
   - Perform a one-time cleanup of the ~10K historical drained rows from the `swarm_task` table to reduce noise.

3. **Retire SwarmTaskBridge and SwarmTaskDrainer:**
   - Once the migration is complete, retire the `SwarmTaskBridge` and `SwarmTaskDrainer` daemons.

4. **Verification Gate:**
   - After migration, monitor the brain-lease completion ratio to ensure it reflects real work and not drained noise.

## Considered Alternatives

1. **Maintaining Both Pipelines:**
   - This would continue the current complexity and potential for data inconsistency.
2. **Partial Migration with Continued Daemons:**
   - While this might reduce immediate complexity, it does not address the root cause of the structural smell and could lead to long-term issues.

## Consequences

1. **Simplified Architecture:**
   - Reducing the number of parallel pipelines simplifies the system architecture.
2. **Improved Data Consistency:**
   - Direct writes to `inference_task` will reduce inconsistencies introduced by intermediate steps.
3. **Reduced Operational Overhead:**
   - Eliminating daemons reduces maintenance and potential points of failure.
4. **Enhanced Monitoring:**
   - By focusing on real work, the brain-lease completion ratio will be a more accurate metric for system health.

## Workplan

1. **Week 1: Analysis and Planning**
   - Conduct a detailed analysis of current data flows and dependencies.
   - Plan the migration strategy, including data mapping and validation steps.

2. **Week 2: Implementation**
   - Implement the changes to `workplan_executor` and Hex Brain Enqueue to write directly to `inference_task`.
   - Update relevant producers in `hex-cli/src/commands/sched.rs` and `workplan_executor`.

3. **Week 3: One-Shot GC**
   - Perform a one-time cleanup of historical drained rows from the `swarm_task` table.

4. **Week 4: Testing and Verification**
   - Test the migration in staging environment.
   - Monitor the brain-lease completion ratio to ensure it reflects real work.

5. **Week 5: Retirement of Daemons**
   - Retire the `SwarmTaskBridge` and `SwarmTaskDrainer` daemons.
   - Conduct final testing and verification.

## Conclusion

By migrating the task management from `swarm_task` to `inference_task`, we can simplify our architecture, improve data consistency, and reduce operational overhead. This approach aligns with our goal of a more maintainable and scalable system.