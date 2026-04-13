//! Composition root — the ONLY file that imports adapters.
//!
//! hex architecture: domain → ports → adapters → composition root.
//! This file wires adapters to ports. Nothing else may import adapters.

mod domain;
mod ports;
mod adapters;

use domain::{Status, Task, TaskId};
use ports::{Command, TaskStore, parse_args};
use adapters::InMemoryTaskStore;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => { eprintln!("{}", e); std::process::exit(1); }
    };

    let mut store = InMemoryTaskStore::new();

    match cmd {
        Command::Add { title, priority } => {
            let id = store.next_id();
            let task = Task::new(&id, &title, priority);
            store.save(task).unwrap();
            println!("Created task {} — {} [{}]", id, title, priority);
        }
        Command::List => {
            let tasks = store.list();
            if tasks.is_empty() {
                println!("No tasks.");
            } else {
                for t in tasks { println!("  {}", t); }
            }
        }
        Command::Start { id } => {
            match store.find_mut(&TaskId(id.clone())) {
                Ok(t) => match t.transition(Status::InProgress) {
                    Ok(()) => println!("Started: {}", t),
                    Err(e) => eprintln!("Error: {}", e),
                },
                Err(e) => eprintln!("Error: {}", e),
            }
        }
        Command::Done { id } => {
            match store.find_mut(&TaskId(id.clone())) {
                Ok(t) => match t.transition(Status::Done) {
                    Ok(()) => println!("Done: {}", t),
                    Err(e) => eprintln!("Error: {}", e),
                },
                Err(e) => eprintln!("Error: {}", e),
            }
        }
        Command::Cancel { id } => {
            match store.find_mut(&TaskId(id.clone())) {
                Ok(t) => match t.transition(Status::Cancelled) {
                    Ok(()) => println!("Cancelled: {}", t),
                    Err(e) => eprintln!("Error: {}", e),
                },
                Err(e) => eprintln!("Error: {}", e),
            }
        }
        Command::Remove { id } => {
            match store.remove(&TaskId(id)) {
                Ok(t) => println!("Removed: {}", t),
                Err(e) => eprintln!("Error: {}", e),
            }
        }
    }
}
