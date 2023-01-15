use anyhow::{bail, ensure, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const DISTRO_ROOTFS_URL: &str =
    "https://cloud-images.ubuntu.com/wsl/jammy/current/ubuntu-jammy-wsl-amd64-wsl.rootfs.tar.gz";
const DISTRO_NAME: &str = "custom-docker-host";
const CONTEXT_NAME: &str = "custom-linux-docker-host";
const DOCKER_HOST: &str = "localhost:22432";

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| panic!("critical error: failed to get home directory"))
}

fn distro_dir_path(name: &str) -> PathBuf {
    let home = home_dir();
    home.join("wsl-distros").join(name)
}

fn find_native_docker_cli_path() -> PathBuf {
    let mut path = home_dir();
    path.extend(Path::new("scoop/shims/docker.exe").components());
    path
}

fn output(args: &[&str], set_context: bool) -> Result<String> {
    // eprintln!("output: {:?}", args);
    let mut cmd = Command::new(args[0]);
    cmd.args(&args[1..]);
    if set_context {
        cmd.env("DOCKER_CONTEXT", CONTEXT_NAME);
    }
    let output = cmd.output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run(args: &[&str], silent: bool, set_context: bool) -> Result<bool> {
    // eprintln!("run: {:?}", args);
    let (stdout, stderr) = if silent {
        (Stdio::null(), Stdio::null())
    } else {
        (Stdio::inherit(), Stdio::inherit())
    };

    let mut cmd = Command::new(args[0]);
    cmd.args(&args[1..]).stdout(stdout).stderr(stderr);
    if set_context {
        cmd.env("DOCKER_CONTEXT", CONTEXT_NAME);
    }

    let status = cmd.spawn()?.wait()?;

    Ok(status.success())
}

fn run_in_wsl(args_in_wsl: &[&str], silent: bool) -> Result<bool> {
    let mut args = vec!["wsl", "-d", DISTRO_NAME, "-e"];
    args.extend(args_in_wsl);
    run(&args, silent, false)
}

fn ensure_native_docker_cli() -> Result<()> {
    let native_cli = find_native_docker_cli_path();
    if !native_cli.exists() {
        bail!(
            "failed to find native docker cli at {}",
            native_cli.display()
        );
    }

    let native_cli = native_cli.to_string_lossy();

    if output(&[&*native_cli, "context", "ls"], false)?.contains(CONTEXT_NAME) {
        return Ok(());
    }

    ensure!(
        run(
            &[
                &*native_cli,
                "context",
                "create",
                CONTEXT_NAME,
                "--docker",
                &format!("host=tcp://{DOCKER_HOST}")
            ],
            true,
            false,
        )?,
        "failed to create docker context"
    );

    Ok(())
}

fn ensure_docker() -> Result<()> {
    if !run_in_wsl(&["which", "docker"], true)? {
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
                false,
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
            false,
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
                &format!(
                    r#"mkdir -p /etc/docker && echo '{{"features":{{"buildkit":true}},"hosts":["{DOCKER_HOST}"]}}' > /etc/docker/daemon.json"#
                )
            ],
            true
        )?,
        "failed to set up buildkit"
    );

    Ok(())
}

fn convert_path(from: &str) -> Result<String> {
    output(
        &["wsl", "-d", DISTRO_NAME, "-e", "wslpath", "-u", from],
        false,
    )
    .map(|s| s.trim().to_string())
}

fn modify_args(args: &mut [String]) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }

    if args[0] == "create" {
        fix_bind_mount_path(args)?;
    }

    Ok(())
}

fn fix_bind_mount_path(args: &mut [String]) -> Result<()> {
    let mut is_mount_option = false;
    for arg in args {
        if is_mount_option {
            is_mount_option = false;
            let mut opts: Vec<String> = arg.split(',').map(|s| s.to_string()).collect();
            for opt in &mut opts {
                if opt.starts_with("source=") {
                    let path = &opt["source=".len()..];
                    let path = convert_path(path)?;
                    *opt = format!("source={path}");
                }
            }
            *arg = opts.join(",");

            continue;
        }

        if arg.trim() == "--mount" {
            is_mount_option = true;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    ensure_native_docker_cli()?;
    ensure_docker()?;

    let mut args: Vec<_> = std::env::args().skip(1).collect();
    modify_args(&mut args)?;

    let native_cli_path = find_native_docker_cli_path();
    let native_cli_path = native_cli_path.to_string_lossy();
    let mut native_args = vec![&*native_cli_path];
    native_args.extend(args.iter().map(|arg| &**arg));
    ensure!(run(&native_args, false, true)?, "docker failed");

    Ok(())
}
