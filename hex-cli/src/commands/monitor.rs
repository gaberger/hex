use serde_json::Value;

// ... existing code ...

async fn get_recent_activity(&self) -> Result<(), Box<dyn std::error::Error>> {
    // ... existing code ...

    let response = reqwest::get("https://api.example.com/tasks")
        .await?
        .json::<Vec<Value>>()
        .await?;

    for task in response {
        if let Some(result) = task.get("result") {
            if result.is_string() && result.as_str().unwrap_or("") == "failure" {
                if let Some(reason) = task.get("reason") {
                    if let Some(reason_str) = reason.as_str() {
                        let truncated_reason = if reason_str.len() > 80 {
                            &reason_str[..80]
                        } else {
                            reason_str
                        };
                        println!("Task failed: {}", truncated_reason);
                    }
                }
            }
        }
    }

    // ... existing code ...

    Ok(())
}

// ... existing code ...