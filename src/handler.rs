// src/handler.rs
// ADR-2026-05-19-0721

use hex_nexus::tool_plan;

/// Executes the tool plan as specified by msg_id=6081.
///
/// # Steps:
/// 1. Parse the tool plan configuration.
/// 2. Validate the inputs.
/// 3. Execute the tool actions.
/// 4. Log the results.
///
/// # Errors:
/// Returns an error if any step in the process fails.
pub fn execute_tool_plan() -> Result<(), String> {
    let plan = tool_plan::get_plan(6081).map_err(|e| format!("Failed to retrieve plan: {}", e))?;
    
    // Validate the inputs
    if !plan.validate() {
        return Err("Plan validation failed".to_string());
    }

    // Execute the tool actions
    for action in plan.actions {
        action.execute().map_err(|e| format!("Action execution failed: {}", e))?;
    }

    // Log the results
    log_results(&plan);

    Ok(())
}

/// Logs the results of the executed tool plan.
fn log_results(plan: &tool_plan::Plan) {
    println!("Tool Plan Execution Summary:");
    for action in &plan.actions {
        println!("Action: {:?}, Status: {:?}", action, action.status());
    }
}