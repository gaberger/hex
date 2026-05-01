use std::time::{Duration, SystemTime};
use chrono::Utc;

// Assuming there is a function to check if the queue is idle
fn is_queue_idle() -> bool {
    // Implementation of queue idleness check
    true
}

// Assuming there is a function to trigger arch-analysis
fn trigger_arch_analysis() {
    // Implementation of triggering arch-analysis
    println!("Arch analysis triggered at {}", Utc::now());
}

struct Scheduler {
    last_arch_analysis_check: Option<SystemTime>,
}

impl Scheduler {
    fn new() -> Self {
        Scheduler {
            last_arch_analysis_check: None,
        }
    }

    fn queue_drain(&mut self) {
        // Existing queue drain logic
        if is_queue_idle() {
            let now = SystemTime::now();
            match self.last_arch_analysis_check {
                Some(last_check) => {
                    if now.duration_since(last_check).unwrap_or(Duration::ZERO) >= Duration::from_secs(2 * 60 * 60) {
                        trigger_arch_analysis();
                        self.last_arch_analysis_check = Some(now);
                    }
                },
                None => {
                    trigger_arch_analysis();
                    self.last_arch_analysis_check = Some(now);
                }
            }
        }
    }
}

// Example usage
fn main() {
    let mut scheduler = Scheduler::new();
    scheduler.queue_drain();
}