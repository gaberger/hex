use std::collections::HashMap;
use crate::domain::{DomainError, Task, TaskId};
use crate::ports::TaskStore;

pub struct InMemoryTaskStore {
    tasks: HashMap<String, Task>,
    next_id: u32,
}

impl InMemoryTaskStore {
    pub fn new() -> Self { Self { tasks: HashMap::new(), next_id: 1 } }
    pub fn next_id(&mut self) -> String { let id = self.next_id.to_string(); self.next_id += 1; id }
}

impl TaskStore for InMemoryTaskStore {
    fn save(&mut self, task: Task) -> Result<(), DomainError> {
        if self.tasks.contains_key(&task.id.0) { return Err(DomainError::DuplicateId(task.id.0.clone())); }
        self.tasks.insert(task.id.0.clone(), task); Ok(())
    }
    fn find(&self, id: &TaskId) -> Result<&Task, DomainError> {
        self.tasks.get(&id.0).ok_or_else(|| DomainError::NotFound(id.0.clone()))
    }
    fn find_mut(&mut self, id: &TaskId) -> Result<&mut Task, DomainError> {
        self.tasks.get_mut(&id.0).ok_or_else(|| DomainError::NotFound(id.0.clone()))
    }
    fn list(&self) -> Vec<&Task> {
        let mut v: Vec<&Task> = self.tasks.values().collect();
        v.sort_by(|a, b| a.priority.cmp(&b.priority).reverse());
        v
    }
    fn remove(&mut self, id: &TaskId) -> Result<Task, DomainError> {
        self.tasks.remove(&id.0).ok_or_else(|| DomainError::NotFound(id.0.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Priority, Status};

    #[test]
    fn save_and_find() {
        let mut s = InMemoryTaskStore::new();
        s.save(Task::new("1", "Test", Priority::Medium)).unwrap();
        assert_eq!(s.find(&TaskId("1".into())).unwrap().title, "Test");
    }
    #[test]
    fn duplicate_rejected() {
        let mut s = InMemoryTaskStore::new();
        s.save(Task::new("1", "A", Priority::Low)).unwrap();
        assert!(s.save(Task::new("1", "B", Priority::High)).is_err());
    }
    #[test]
    fn find_mut_transition() {
        let mut s = InMemoryTaskStore::new();
        s.save(Task::new("1", "T", Priority::High)).unwrap();
        s.find_mut(&TaskId("1".into())).unwrap().transition(Status::InProgress).unwrap();
        assert_eq!(s.find(&TaskId("1".into())).unwrap().status, Status::InProgress);
    }
    #[test]
    fn list_sorted_by_priority() {
        let mut s = InMemoryTaskStore::new();
        s.save(Task::new("1", "Low", Priority::Low)).unwrap();
        s.save(Task::new("2", "Crit", Priority::Critical)).unwrap();
        let list = s.list();
        assert_eq!(list[0].priority, Priority::Critical);
    }
    #[test]
    fn remove_works() {
        let mut s = InMemoryTaskStore::new();
        s.save(Task::new("1", "Bye", Priority::Low)).unwrap();
        s.remove(&TaskId("1".into())).unwrap();
        assert!(s.find(&TaskId("1".into())).is_err());
    }
    #[test]
    fn not_found() {
        assert!(InMemoryTaskStore::new().find(&TaskId("x".into())).is_err());
    }
}
