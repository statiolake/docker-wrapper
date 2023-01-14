use anyhow::{ensure, Result};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

const DISTRO_ROOTFS_URL: &str =
    "https://cloud-images.ubuntu.com/wsl/jammy/current/ubuntu-jammy-wsl-amd64-wsl.rootfs.tar.gz";
const DISTRO_NAME: &str = "docker-host";

fn distro_dir_path(name: &str) -> PathBuf {
    let home =
        dirs::home_dir().unwrap_or_else(|| panic!("critical error: failed to get home directory"));
    home.join("wsl-distros").join(name)
}

fn run(cmd: &[&str], silent: bool) -> Result<bool> {
    let (stdout, stderr) = if silent {
        (Stdio::null(), Stdio::null())
    } else {
        (Stdio::inherit(), Stdio::inherit())
    };
    let status = Command::new(cmd[0])
        .args(&cmd[1..])
        .stdout(stdout)
        .stderr(stderr)
        .spawn()?
        .wait()?;
    Ok(status.success())
}

fn run_in_wsl(cmd_in_wsl: &[&str], silent: bool) -> Result<bool> {
    let mut cmd = vec!["wsl", "-d", DISTRO_NAME, "-e"];
    cmd.extend(cmd_in_wsl);
    run(&cmd, silent)
}

fn ensure_docker() -> Result<()> {
    if !run_in_wsl(&["docker", "version"], true)? {
        setup_docker_distro()?;
    }
    run_in_wsl(&["/sbin/service", "docker", "start"], true)?;

    Ok(())
}

fn setup_docker_distro() -> Result<()> {
    eprintln!("setup Ubuntu 22.04 from '{}'...", DISTRO_ROOTFS_URL);
    download_and_import_rootfs()?;

    eprintln!("setup docker engine...");
    setup_docker_on_distro()?;

    Ok(())
}

fn download_and_import_rootfs() -> Result<()> {
    // TODO
    let path = distro_dir_path(DISTRO_NAME);
    let distro_root_path = path.join("root");
    let download_path = path.join("rootfs.tar.gz");

    fs::create_dir_all(&distro_root_path)?;

    if !download_path.exists() {
        ensure!(
            run(
                &[
                    "curl",
                    "-L",
                    DISTRO_ROOTFS_URL,
                    "-o",
                    &download_path.display().to_string(),
                ],
                false
            )?,
            "failed to download rootfs"
        );
    }

    ensure!(
        run(
            &[
                "wsl",
                "--import",
                DISTRO_NAME,
                &distro_root_path.display().to_string(),
                &download_path.display().to_string()
            ],
            false
        )?,
        "failed to import distro"
    );

    Ok(())
}

fn setup_docker_on_distro() -> Result<()> {
    ensure!(
        run_in_wsl(
            &["sh", "-c", "curl -fsSL https://get.docker.com/ | sh"],
            false
        )?,
        "failed to install docker engine"
    );

    ensure!(
        run_in_wsl(
            &[
                "sh",
                "-c",
                r#"mkdir -p ~/.docker && echo '{"detachKeys":"ctrl-^"}' > ~/.docker/config"#
            ],
            true
        )?,
        "failed to set up detach keys"
    );

    ensure!(
        run_in_wsl(
            &[
                "sh",
                "-c",
                r#"mkdir -p /etc/docker && echo '{"features":{"buildkit":true}}' > /etc/docker/daemon.json"#
            ],
            true
        )?,
        "failed to set up buildkit"
    );

    Ok(())
}

fn main() -> Result<()> {
    ensure_docker()?;

    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut cmd = vec!["docker"];
    cmd.extend(args.iter().map(|arg| &**arg));
    run_in_wsl(&cmd, false)?;

    Ok(())
}
