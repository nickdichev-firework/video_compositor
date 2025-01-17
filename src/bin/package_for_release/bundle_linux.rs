use anyhow::{anyhow, Result};
use fs_extra::dir::{self, CopyOptions};
use log::info;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::utils;

const X86_TARGET: &str = "x86_64-unknown-linux-gnu";

pub fn bundle_linux_app() -> Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );
    let root_dir_str = env!("CARGO_MANIFEST_DIR");
    let root_dir: PathBuf = root_dir_str.into();
    let release_dir = root_dir.join("target/x86_64-unknown-linux-gnu/release");
    let tmp_dir = root_dir.join("video_compositor");

    info!("Build main_process binary.");
    utils::cargo_build("main_process", X86_TARGET)?;
    info!("Build process_helper binary.");
    utils::cargo_build("process_helper", X86_TARGET)?;

    info!("Create {} directory", tmp_dir.display());
    fs::create_dir_all(tmp_dir.clone())?;

    info!("Copy main_process binary.");
    fs::copy(
        release_dir.join("main_process"),
        tmp_dir.join("video_compositor_main"),
    )?;
    info!("Copy process_helper binary.");
    fs::copy(
        release_dir.join("process_helper"),
        tmp_dir.join("video_compositor_process_helper"),
    )?;

    info!("Copy wrapper script.");
    fs::copy(
        root_dir.join("scripts/compositor_runtime_wrapper.sh"),
        tmp_dir.join("video_compositor"),
    )?;

    info!(
        "Copy lib directory. {:?} {:?}",
        release_dir.join("lib"),
        tmp_dir.join("lib"),
    );

    dir::copy(release_dir.join("lib"), tmp_dir, &CopyOptions::default())?;

    info!("Create tar.gz archive.");
    let exit_code = Command::new("tar")
        .args([
            "-C",
            root_dir_str,
            "-czvf",
            "video_compositor_linux_x86_64.tar.gz",
            "video_compositor",
        ])
        .spawn()?
        .wait()?
        .code();
    if exit_code != Some(0) {
        return Err(anyhow!("Command tar failed with exit code {:?}", exit_code));
    }

    Ok(())
}
