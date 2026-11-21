//! Running Mojang's built-in data generator (`net.minecraft.data.Main`) from
//! the imported client jar. Its `--reports` output (registries, block states,
//! packets, commands, item components) feeds `cargo xtask codegen`.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

use crate::import::Imported;
use crate::java::Java;
use crate::step;

const MAIN_CLASS: &str = "net.minecraft.data.Main";

#[derive(Debug)]
pub enum StepError {
    Io(io::Error),
    /// The JVM exited unsuccessfully; its output is in `log`.
    Failed {
        status: ExitStatus,
        log: PathBuf,
    },
    /// The JVM succeeded but an expected output file is missing.
    MissingOutput(PathBuf),
}

impl From<io::Error> for StepError {
    fn from(e: io::Error) -> Self {
        StepError::Io(e)
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepError::Io(e) => write!(f, "{e}"),
            StepError::Failed { status, log } => {
                write!(f, "Java exited with {status}; see {}", log.display())
            }
            StepError::MissingOutput(p) => write!(f, "{} was not produced", p.display()),
        }
    }
}

impl std::error::Error for StepError {}

/// Where the reports end up: `<cache>/versions/<id>/generated/reports`.
pub fn reports_dir(imported: &Imported) -> PathBuf {
    imported.dir.join("generated").join("reports")
}

fn reports_key(imported: &Imported) -> String {
    step::key(&["datagen-reports-v1", &imported.key()])
}

/// Whether the reports for this import already exist (no JVM needed).
pub fn is_current(imported: &Imported) -> bool {
    step::is_current(&imported.dir.join("generated"), &reports_key(imported))
}

/// Runs `--reports` unless its output for these inputs already exists.
/// Returns the reports folder and whether the generator ran.
pub fn run_reports(java: &Java, imported: &Imported) -> Result<(PathBuf, bool), StepError> {
    let out = imported.dir.join("generated");
    let key = reports_key(imported);
    let ran = step::run(&out, &key, |tmp| {
        let class_path = std::env::join_paths(imported.class_path())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        run_logged(
            Command::new(&java.path)
                .arg("-cp")
                .arg(class_path)
                .arg(MAIN_CLASS)
                .arg("--reports")
                .arg("--output")
                .arg(tmp),
            imported,
            "datagen",
        )?;
        let expected = tmp.join("reports").join("registries.json");
        if !expected.is_file() {
            return Err(StepError::MissingOutput(expected));
        }
        Ok(())
    })?;
    Ok((reports_dir(imported), ran))
}

/// Runs a JVM with its output in `<dir>/logs/<name>.log` and its working
/// directory in `<dir>/work` (vanilla's logger writes `logs/` there).
pub(crate) fn run_logged(
    cmd: &mut Command,
    imported: &Imported,
    name: &str,
) -> Result<(), StepError> {
    let logs = imported.dir.join("logs");
    let work = imported.dir.join("work");
    std::fs::create_dir_all(&logs)?;
    std::fs::create_dir_all(&work)?;
    let log = logs.join(format!("{name}.log"));
    let file = File::create(&log)?;
    let status = cmd
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(StepError::Failed { status, log })
    }
}
