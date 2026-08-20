# Kanban Panel Enhancement: Hiding Drained Orphaned Tasks

## Background
The Kanban panel currently displays swarm_task rows with results starting with 'auto-drained: orphaned (no agent claim within 24h)'. These tasks are released by the SwarmTaskDrainer as part of its release valve mechanism and do not represent real work. This causes significant UX harm, as operators see thousands of pending rows that will never run, without clear visibility into why these tasks exist.

## Objective
1. **Filter Rule**: Implement a filter to hide swarm_task rows in both the Failed lane and Ready lane where the result starts with 'auto-drained: orphaned' OR (status='failed' AND agent_id=''').
2. **Single Roll-up Banner**: Add a banner at the top of the Failed lane that states 'N tasks auto-drained (no consumer)'. This banner should be clickable to reveal the hidden tasks but should default to a collapsed state.
3. **Data Retention**: Ensure that these rows remain in the STDB; CTO is handling the garbage collection migration separately.
4. **Dashboard File Location**: The dashboard file is located at hex-nexus/assets/.
5. **Binary Rebuild**: After making changes to the asset, rebuild the hex-nexus binary.

## Implementation Steps

### 1. Filter Rule
- **Location**: Modify the Kanban panel's filter logic in `hex-nexus/src/dashboard/kanban_panel.rs`.
- **Condition**: Update the filter condition to exclude rows where result starts with 'auto-drained: orphaned' OR (status='failed' AND agent_id=''').
  ```rust
  if task.result.starts_with("auto-drained: orphaned") || (task.status == "failed" && task.agent_id.is_empty()) {
      continue;
  }
  ```

### 2. Single Roll-up Banner
- **Location**: Add a new banner component in `hex-nexus/src/dashboard/components/banner.rs`.
- **Implementation**:
  ```rust
  pub fn auto_drained_banner(tasks: Vec<SwarmTask>) -> Html {
      let auto_drained_count = tasks.iter().filter(|t| t.result.starts_with("auto-drained: orphaned")).count();
      html! {
          <div class="banner" onclick={ move |_| toggle_visibility() }>
              { format!("{} tasks auto-drained (no consumer)", auto_drained_count) }
          </div>
      }
  }
  ```

- **Visibility Toggle**:
  ```rust
  fn toggle_visibility() {
      // Logic to show/hide the hidden tasks
  }
  ```

### 3. Data Retention
- Ensure that the filter only affects the display and not the underlying data in STDB.

### 4. Dashboard File Location
- The Kanban panel is located at `hex-nexus/assets/kanban_panel.html`.

### 5. Binary Rebuild
- After making changes, rebuild the hex-nexus binary:
  ```bash
  cargo build --release
  ```

## Highest ROI: The Banner
Even if the filter rule implementation is delayed, adding the banner will significantly improve the Kanban panel's legibility by providing a clear summary of auto-drained tasks that can be expanded for more detailed view.

## Conclusion
By implementing these changes, we address the UX harm caused by displaying orphaned tasks and provide operators with a clearer understanding of the task status on the Kanban panel.