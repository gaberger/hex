pub trait HelloService {
   fn greet(&self) -> String;
}

impl HelloService for HelloServiceImpl {
    fn greet(&self) -> String {
        "Hello, World!".to_string()
    }
}