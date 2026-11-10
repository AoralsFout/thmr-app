//! Optional local measurement of the content position actually submitted to the terminal.
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
pub struct Trace(BufWriter<File>);
impl Trace {
    pub fn new() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("THMR_TUI_TRACE_DIR")?);
        fs::create_dir_all(&path).ok()?;
        Some(Self(BufWriter::new(
            File::create(path.join("tui-draw.csv")).ok()?,
        )))
    }
    pub fn draw(&mut self, epoch: u64, sample: u64) {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let _ = writeln!(self.0, "{},{},{}", time, epoch, sample);
    }
}
