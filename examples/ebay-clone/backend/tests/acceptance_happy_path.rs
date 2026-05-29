```rust
// examples/ebay-clone/backend/tests/acceptance_happy_path.rs

use fantoccini::{ClientBuilder, Locator};
use std::time::Duration;

#[tokio::test]
async fn happy_path() {
    // Initialize the browser client
    let mut c = ClientBuilder::native()
        .connect("http://localhost:4444")
        .await
        .expect("Failed to connect to WebDriver");

    // Register user A
    c.goto("http://localhost:8000/register").await.expect("Failed to navigate to register page");
    c.find(Locator::Css(".username")).await.expect("Failed to find username field")
        .click()
        .await.expect("Failed to click username field")
        .send_keys("userA")
        .await.expect("Failed to send keys to username field");

    c.find(Locator::Css(".password")).await.expect("Failed to find password field")
        .click()
        .await.expect("Failed to click password field")
        .send_keys("password123")
        .await.expect("Failed to send keys to password field");

    c.find(Locator::Css(".register-button")).await.expect("Failed to find register button")
        .click()
        .await.expect("Failed to click register button");

    // Register user B
    c.goto("http://localhost:8000/register").await.expect("Failed to navigate to register page");
    c.find(Locator::Css(".username")).await.expect("Failed to find username field")
        .click()
        .await.expect("Failed to click username field")
        .send_keys("userB")
        .await.expect("Failed to send keys to username field");

    c.find(Locator::Css(".password")).await.expect("Failed to find password field")
        .click()
        .await.expect("Failed to click password field")
        .send_keys("password123")
        .await.expect("Failed to send keys to password field");

    c.find(Locator::Css(".register-button")).await.expect("Failed to find register button")
        .click()
        .await.expect("Failed to click register button");

    // User A posts a listing
    c.goto("http://localhost:8000/post_listing").await.expect("Failed to navigate to post listing page");
    c.find(Locator::Css(".title")).await.expect("Failed to find title field")
        .click()
        .await.expect("Failed to click title field")
        .send_keys("Example Item")
        .await.expect("Failed to send keys to title field");

    c.find(Locator::Css(".description")).await.expect("Failed to find description field")
        .click()
        .await.expect("Failed to click description field")
        .send_keys("A great item for sale!")
        .await.expect("Failed to send keys to description field");

    c.find(Locator::Css(".price")).await.expect("Failed to find price field")
        .click()
        .await.expect("Failed to click price field")
        .send_keys("10.00")
        .await.expect("Failed to send keys to price field");

    c.find(Locator::Css(".post-button")).await.expect("Failed to find post button")
        .click()
        .await.expect("Failed to click post button");

    // User B places a bid
    c.goto("http://localhost:8000/listings").await.expect("Failed to navigate to listings page");
    let listing = c.find(Locator::Css(".listing")).await.expect("Failed to find listing element");
    listing.click().await.expect("Failed to click on listing");

    c.find(Locator::Css(".bid-input")).await.expect("Failed to find bid input field")
        .click()
        .await.expect("Failed to click bid input field")
        .send_keys("15.00")
        .await.expect("Failed to send keys to bid input field");

    c.find(Locator::Css(".place-bid-button")).await.expect("Failed to find place bid button")
        .click()
        .await.expect("Failed to click place bid button");

    // Wait for auction close
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Assert winner=B in MyWon
    c.goto("http://localhost:8000/my_won").await.expect("Failed to navigate to my won page");
    let winner = c.find(Locator::Css(".winner")).await.expect("Failed to find winner element");
    assert_eq!(winner.text().await.expect("Failed to get winner text"), "userB", "Winner should be userB");

    // Assert sold-to-B in MyListings
    c.goto("http://localhost:8000/my_listings").await.expect("Failed to navigate to my listings page");
    let sold_to = c.find(Locator::Css(".sold-to")).await.expect("Failed to find sold