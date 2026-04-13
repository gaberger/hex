use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
pub struct TaskId(pub String);

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl Status {
    pub fn can_transition_to(&self, new_status: &Status) -> bool {
        match (self, new_status) {
            (Status::Todo, Status::InProgress) => true,
            (Status::Todo, Status::Cancelled) => true,
            (Status::InProgress, Status::Done) => true,
            (Status::InProgress, Status::Cancelled) => true,
            (Status::Done, _) => false,
            (Status::Cancelled, _) => false,
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: Status,
    pub priority: Priority,
}

impl Task {
    pub fn transition(&mut self, new_status: Status) -> Result<(), DomainError> {
        if self.status.can_transition_to(&new_status) {
            self.status = new_status;
            Ok(())
        } else {
            Err(DomainError::InvalidTransition)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DomainError {
    InvalidTransition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_to_in_progress() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
        };
        assert!(task.transition(Status::InProgress).is_ok());
    }

    #[test]
    fn test_todo_to_cancelled() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
        };
        assert!(task.transition(Status::Cancelled).is_ok());
    }

    #[test]
    fn test_in_progress_to_done() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::InProgress,
            priority: Priority::Medium,
        };
        assert!(task.transition(Status::Done).is_ok());
    }

    #[test]
    fn test_in_progress_to_cancelled() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::InProgress,
            priority: Priority::Medium,
        };
        assert!(task.transition(Status::Cancelled).is_ok());
    }

    #[test]
    fn test_done_no_transition() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Done,
            priority: Priority::Medium,
        };
        assert_eq!(task.transition(Status::Todo), Err(DomainError::InvalidTransition));
    }

    #[test]
    fn test_cancelled_no_transition() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Cancelled,
            priority: Priority::Medium,
        };
        assert_eq!(task.transition(Status::Todo), Err(DomainError::InvalidTransition));
    }

    #[test]
    fn test_invalid_transition() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
        };
        assert_eq!(task.transition(Status::Done), Err(DomainError::InvalidTransition));
    }

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Low < Priority::Medium);
        assert!(Priority::Medium < Priority::High);
        assert!(Priority::High < Priority::Critical);
    }
}
