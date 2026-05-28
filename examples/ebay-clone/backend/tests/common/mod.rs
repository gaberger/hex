use std::net::{SocketAddr, TcpListener};
use spacetime_modules::marketplace;
use spacetimedb::auth::{AuthorizationToken, KeyPair};

pub struct TestSetup {
    pub addr: SocketAddr,
    pub auth_token: AuthorizationToken,
}

impl Drop for TestSetup {
    fn drop(&mut self) {
        // Teardown logic here if needed
    }
}

pub async fn setup() -> TestSetup {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");
    let addr = listener.local_addr().unwrap();

    // Initialize STDB and publish the marketplace module
    spacetimedb::test::init();
    spacetimedb::test::publish_module(&marketplace::module()).await;

    // Generate a key pair and create an auth token
    let key_pair = KeyPair::new_ed25519();
    let auth_token = AuthorizationToken::from(key_pair.public_key());

    TestSetup {
        addr,
        auth_token,
    }
}

pub fn assert_status_code(response: reqwest::Response, expected: u16) {
    assert_eq!(response.status().as_u16(), expected);
}

// docs/workplans/feat-ebay-mvp.json
// hex-nexus/backend/tests/common/mod.rs