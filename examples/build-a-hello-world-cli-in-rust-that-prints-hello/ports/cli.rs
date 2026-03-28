pub trait CliPort {
   fn read_input(&self) -> anyhow::Result<String>;
    fn write_output(&self, message: &str) -> anyhow::Result<()>;
}