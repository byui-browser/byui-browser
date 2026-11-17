//! On-disk appearance preference for the desktop shell.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::context_menu::Appearance;

pub(crate) fn load(path: &Path) -> io::Result<Appearance> {
    let value = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Appearance::System),
        Err(error) => return Err(error),
    };
    match value.trim() {
        "system" => Ok(Appearance::System),
        "light" => Ok(Appearance::Light),
        "dark" => Ok(Appearance::Dark),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unrecognized appearance preference",
        )),
    }
}

pub(crate) fn save(path: &Path, appearance: Appearance) -> io::Result<()> {
    let value = match appearance {
        Appearance::System => "system\n",
        Appearance::Light => "light\n",
        Appearance::Dark => "dark\n",
    };

    // Stage the update alongside the destination so a failed write leaves the
    // previous selection intact. Exclusive creation also protects other instances.
    static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
    let suffix = NEXT_WRITE.fetch_add(1, Ordering::Relaxed);
    let pending = path.with_extension(format!("{}.{}.tmp", std::process::id(), suffix));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let written = file
        .write_all(value.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&pending, path));
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result
}
