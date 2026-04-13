use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskId(pub String);

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl Status {
    pub fn can_transition_to(&self, next: Status) -> bool {
        matches!(
            (self, next),
            (Status::Todo, Status::InProgress)
                | (Status::Todo, Status::Cancelled)
                | (Status::InProgress, Status::Done)
                | (Status::InProgress, Status::Todo)
                | (Status::InProgress, Status::Cancelled)
        )
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Todo => write!(f, "todo"),
            Status::InProgress => write!(f, "in-progress"),
            Status::Done => write!(f, "done"),
            Status::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Priority::Low => write!(f, "low"),
            Priority::Medium => write!(f, "medium"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: Status,
    pub priority: Priority,
}

impl Task {
    pub fn new(id: &str, title: &str, priority: Priority) -> Self {
        Self {
            id: TaskId(id.to_string()),
            title: title.to_string(),
            status: Status::Todo,
            priority,
        }
    }

    pub fn transition(&mut self, new_status: Status) -> Result<(), String> {
        if self.status.can_transition_to(new_status) {
            self.status = new_status;
            Ok(())
        } else {
            Err(format!("cannot transition {} → {}", self.status, new_status))
        }
    }
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} ({}, {})", self.id, self.title, self.status, self.priority)
    }
}

#[derive(Debug)]
pub enum DomainError {
    NotFound(String),
    DuplicateId(String),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DomainError::NotFound(id) => write!(f, "not found: {}", id),
            DomainError::DuplicateId(id) => write!(f, "duplicate id: {}", id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_task_is_todo() {
        assert_eq!(Task::new("1", "t", Priority::Low).status, Status::Todo);
    }

    #[test]
    fn todo_to_in_progress() {
        let mut t = Task::new("1", "t", Priority::Medium);
        assert!(t.transition(Status::InProgress).is_ok());
        assert_eq!(t.status, Status::InProgress);
    }

    #[test]
    fn in_progress_to_done() {
        let mut t = Task::new("1", "t", Priority::High);
        t.transition(Status::InProgress).unwrap();
        assert!(t.transition(Status::Done).is_ok());
    }

    #[test]
    fn todo_to_done_invalid() {
        let mut t = Task::new("1", "t", Priority::Low);
        assert!(t.transition(Status::Done).is_err());
        assert_eq!(t.status, Status::Todo);
    }

    #[test]
    fn done_is_terminal() {
        let mut t = Task::new("1", "t", Priority::Low);
        t.transition(Status::InProgress).unwrap();
        t.transition(Status::Done).unwrap();
        assert!(t.transition(Status::InProgress).is_err());
    }

    #[test]
    fn cancel_from_todo() {
        let mut t = Task::new("1", "t", Priority::Low);
        assert!(t.transition(Status::Cancelled).is_ok());
    }

    #[test]
    fn cancel_from_in_progress() {
        let mut t = Task::new("1", "t", Priority::High);
        t.transition(Status::InProgress).unwrap();
        assert!(t.transition(Status::Cancelled).is_ok());
    }

    #[test]
    fn revert_to_todo() {
        let mut t = Task::new("1", "t", Priority::Medium);
        t.transition(Status::InProgress).unwrap();
        assert!(t.transition(Status::Todo).is_ok());
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::Low < Priority::Critical);
        assert!(Priority::Medium < Priority::High);
    }

    #[test]
    fn display_includes_all_fields() {
        let t = Task::new("42", "Ship it", Priority::Critical);
        let s = format!("{}", t);
        assert!(s.contains("42") && s.contains("Ship it") && s.contains("critical"));
    }
}
