// src/domain/mod.rs

#[derive(Debug, PartialEq, Eq)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(id: String) -> Self {
        TaskId(id)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl Status {
    pub fn can_transition_to(&self, next_status: &Status) -> bool {
        use Status::*;
        match (self, next_status) {
            (Todo, InProgress) | (Todo, Cancelled) => true,
            (InProgress, Todo) | (InProgress, Done) | (InProgress, Cancelled) => true,
            (Done, _) => false,
            (Cancelled, _) => false,
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Ord, PartialOrd)]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug)]
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
            Err(DomainError::InvalidStatusTransition)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DomainError {
    InvalidStatusTransition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_todo_to_in_progress() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::InProgress).is_ok());
        assert_eq!(task.status, Status::InProgress);
    }

    #[test]
    fn test_transition_todo_to_cancelled() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::Cancelled).is_ok());
        assert_eq!(task.status, Status::Cancelled);
    }

    #[test]
    fn test_transition_in_progress_to_todo() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::InProgress,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::Todo).is_ok());
        assert_eq!(task.status, Status::Todo);
    }

    #[test]
    fn test_transition_in_progress_to_done() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::InProgress,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::Done).is_ok());
        assert_eq!(task.status, Status::Done);
    }

    #[test]
    fn test_transition_in_progress_to_cancelled() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::InProgress,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::Cancelled).is_ok());
        assert_eq!(task.status, Status::Cancelled);
    }

    #[test]
    fn test_transition_done_to_in_progress() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Done,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::InProgress).is_err());
    }

    #[test]
    fn test_transition_cancelled_to_in_progress() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Cancelled,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::InProgress).is_err());
    }

    #[test]
    fn test_transition_same_status() {
        let mut task = Task {
            id: TaskId("1".to_string()),
            title: "Test Task".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
        };

        assert!(task.transition(Status::Todo).is_err());
    }
}
