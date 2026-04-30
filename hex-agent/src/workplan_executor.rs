use std::fs;

fn validate_generated_code(code: &str) -> Result<(), String> {
    // Placeholder validation logic
    if code.is_empty() {
        Err("Generated code is empty".to_string())
    } else {
        Ok(())
    }
}

fn write_file(path: &str, content: &str) -> Result<(), String> {
    match validate_generated_code(content) {
        Ok(_) => fs::write(path, content).map_err(|e| e.to_string()),
        Err(e) => Err(format!("Validation failed: {}", e)),
    }
}

fn apply_workplan(workplan: &Workplan) -> Result<(), String> {
    for task in &workplan.tasks {
        let generated_code = generate_code(task)?;
        match validate_generated_code(&generated_code) {
            Ok(_) => fs::write(&task.output_path, &generated_code).map_err(|e| e.to_string()),
            Err(e) => return Err(format!("Validation failed: {}", e)),
        }
    }
    Ok(())
}