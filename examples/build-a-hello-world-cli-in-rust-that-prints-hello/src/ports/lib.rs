pub trait PrinterPort {
    fn print(&self, content: &str) -> Result<(), Box<dyn std::error::Error>>;
}