use crate::core::domain::*;
use async_trait::async_trait;

/// UserRepoPort defines CRUD operations against the user table via STDB.
///
/// This trait is Send+Sync to allow concurrent access from multiple threads.
///
/// # Examples
///
/// ```
/// // Example usage of UserRepoPort would go here
/// ```
#[async_trait]
pub trait UserRepoPort: Send + Sync {
    /// Creates a new user in the repository.
    ///
    /// # Arguments
    /// * `user` - A reference to a [CreateUserInput] object representing the user to be created.
    ///
    /// # Returns
    /// * A Result containing either the ID of the newly created user or an error.
    async fn create_user(&self, user: &CreateUserInput) -> Result<UserId, DomainError>;

    /// Retrieves a user by their unique identifier.
    ///
    /// # Arguments
    /// * `user_id` - The [UserId] of the user to retrieve.
    ///
    /// # Returns
    /// * A Result containing either the [User] object if found, or an error.
    async fn get_user_by_id(&self, user_id: UserId) -> Result<User, DomainError>;

    /// Updates an existing user in the repository.
    ///
    /// # Arguments
    /// * `user` - A reference to a [User] object representing the updated user information.
    ///
    /// # Returns
    /// * A Result indicating success or failure of the update operation.
    async fn update_user(&self, user: &User) -> Result<(), DomainError>;

    /// Deletes a user from the repository by their unique identifier.
    ///
    /// # Arguments
    /// * `user_id` - The [UserId] of the user to delete.
    ///
    /// # Returns
    /// * A Result indicating success or failure of the deletion operation.
    async fn delete_user(&self, user_id: UserId) -> Result<(), DomainError>;
}

/// DTO for creating a new user.
///
/// This struct is used as input to the `create_user` method in [UserRepoPort].
pub struct CreateUserInput {
    pub username: String,
    pub email: String,
    pub password_hash: String,
    // Add other necessary fields here
}

// docs/specs/ebay-spec-001