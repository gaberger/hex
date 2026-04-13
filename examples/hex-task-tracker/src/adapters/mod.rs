// Define your Task and TaskId structs here
#[derive(Debug, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub description: String,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct TaskId(pub u32);
