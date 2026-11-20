use std::process::Command;

use anyhow::{Context, Result};

use crate::model::LaunchTarget;

/// Windows error: "The requested operation requires elevation."
const ERROR_ELEVATION_REQUIRED: i32 = 740;

/// Starts the target. Returns the process ID when we started the process
/// ourselves (direct executables); launcher URIs and shortcuts don't tell us.
pub fn launch(target: &LaunchTarget) -> Result<Option<u32>> {
    match target {
        LaunchTarget::Uri(uri) => {
            open::that_detached(uri).with_context(|| format!("opening {uri}"))?;
        }
        LaunchTarget::Shell(path) => {
            open::that_detached(path).with_context(|| format!("opening {}", path.display()))?;
        }
        LaunchTarget::Exe { path, args, working_dir } => {
            let dir = working_dir.clone().or_else(|| path.parent().map(|p| p.to_path_buf()));
            let mut cmd = Command::new(path);
            cmd.args(args);
            if let Some(dir) = &dir {
                cmd.current_dir(dir);
            }
            match cmd.spawn() {
                Ok(child) => return Ok(Some(child.id())),
                // Executables that demand admin rights cannot be started with
                // CreateProcess; ShellExecute shows the UAC prompt instead.
                Err(e) if e.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) && args.is_empty() => {
                    open::that_detached(path).with_context(|| format!("starting {}", path.display()))?;
                }
                Err(e) => return Err(e).with_context(|| format!("starting {}", path.display())),
            }
        }
    }
    Ok(None)
}
