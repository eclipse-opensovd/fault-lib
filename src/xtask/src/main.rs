/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 */

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, exit};

fn main() {
    let mut args = env::args().skip(1); // skip the binary name

    // println!("{:?}", args.next());
    let Some(command) = args.next() else {
        print_usage_and_exit();
    };

    // Split into env vars (KEY=VALUE) and passthrough args
    let mut cli_env_vars = HashMap::new();
    let mut passthrough_args = Vec::new();

    for arg in args {
        if let Some((key, value)) = arg.split_once('=') {
            cli_env_vars.insert(key.to_string(), value.to_string());
        } else {
            passthrough_args.push(arg);
        }
    }

    let envs = HashMap::new();

    match command.as_str() {
        "build" => {
            debug_build(envs, cli_env_vars, &passthrough_args);
        }
        "clippy" => {
            clippy(envs, cli_env_vars, &passthrough_args);
        }
        "run" => {
            run_build(
                "debug_build",
                &["run"],
                envs,
                cli_env_vars,
                &passthrough_args,
            );
        }
        "build:release" => {
            run_build(
                "release_build",
                &["build", "--release"],
                envs,
                cli_env_vars,
                &passthrough_args,
            );
        }
        "run:release" => {
            run_build(
                "release_build",
                &["run", "--release"],
                envs,
                cli_env_vars,
                &passthrough_args,
            );
        }
        "build:test" | "test" => {
            test(envs, cli_env_vars, &passthrough_args);
        }
        "build:qnx_x86_64" => {
            run_build(
                "",
                &["+qnx7.1_rust", "build", "--target", "x86_64-pc-nto-qnx710"],
                envs,
                cli_env_vars,
                &passthrough_args,
            );
        }
        "build:qnx_arm" => {
            run_build(
                "",
                &[
                    "+qnx7.1_rust",
                    "build",
                    "--target",
                    "aarch64-unknown-nto-qnx710",
                ],
                envs,
                cli_env_vars,
                &passthrough_args,
            );
        }
        "check_lic" => {
            check_license_header();
        }
        "check" => {
            check_license_header();
            run_command(
                &["fmt", "--", "--check"],
                &HashMap::default(),
                &passthrough_args,
                Some("Wrong formatting@"),
            );
            debug_build(envs.clone(), cli_env_vars.clone(), &passthrough_args);
            clippy(envs.clone(), cli_env_vars.clone(), &passthrough_args);
            test(envs, cli_env_vars, &passthrough_args);
        }
        "fmt" => {
            run_command(&["fmt"], &HashMap::default(), &passthrough_args, None);
        }
        "fmt:check" => {
            run_command(
                &["fmt", "--", "--check"],
                &HashMap::default(),
                &passthrough_args,
                Some("Wrong formatting"),
            );
        }
        "miri" => {
            miri(envs, cli_env_vars, &passthrough_args);
        }
        "coverage" => {
            coverage(cli_env_vars, &passthrough_args);
        }
        _ => print_usage_and_exit(),
    }
}

fn clippy(
    envs: HashMap<String, String>,
    cli_env_vars: HashMap<String, String>,
    passthrough_args: &[String],
) {
    run_build(
        "clippy",
        &["clippy", "--all-targets", "--all-features"],
        envs,
        cli_env_vars,
        passthrough_args,
    );
}

fn test(
    envs: HashMap<String, String>,
    cli_env_vars: HashMap<String, String>,
    passthrough_args: &[String],
) {
    run_build(
        "test_build",
        &["test"],
        envs,
        cli_env_vars,
        passthrough_args,
    );
}

fn miri(
    mut envs: HashMap<String, String>,
    cli_env_vars: HashMap<String, String>,
    passthrough_args: &[String],
) {
    envs.insert("MIRIFLAGS".into(), "-Zmiri-disable-isolation".into());
    // Miri cannot interpret IPC syscalls (integration_tests) or proc-macro
    // build scripts (xtask), so both are excluded.
    let mut args: Vec<String> = vec![
        "+nightly".into(),
        "miri".into(),
        "test".into(),
        "--workspace".into(),
        "--exclude".into(),
        "integration_tests".into(),
        "--exclude".into(),
        "xtask".into(),
    ];
    args.extend(passthrough_args.iter().cloned());

    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();

    for (k, v) in cli_env_vars {
        envs.insert(k, v);
    }

    run_command(&args_ref, &envs, &[], None);
}

fn coverage(cli_env_vars: HashMap<String, String>, passthrough_args: &[String]) {
    let mut envs = HashMap::new();
    for (k, v) in cli_env_vars {
        envs.insert(k, v);
    }

    let mut args: Vec<String> = vec![
        "llvm-cov".into(),
        "--workspace".into(),
        "--exclude".into(),
        "xtask".into(),
        "--ignore-filename-regex".into(),
        "test_utils|dfm_test_utils".into(),
    ];
    args.extend(passthrough_args.iter().cloned());

    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    run_command(&args_ref, &envs, &[], None);
}

fn debug_build(
    envs: HashMap<String, String>,
    cli_env_vars: HashMap<String, String>,
    passthrough_args: &[String],
) {
    run_build(
        "debug_build",
        &["build"],
        envs,
        cli_env_vars,
        passthrough_args,
    );
}

fn run_build(
    target_dir: &str,
    cargo_args: &[&str],
    mut default_envs: HashMap<String, String>,
    cli_envs: HashMap<String, String>,
    extra_args: &[String],
) {
    // Set target dir
    default_envs.insert("CARGO_TARGET_DIR".into(), format!("target/{target_dir}"));

    // CLI overrides
    for (k, v) in cli_envs {
        default_envs.insert(k, v);
    }

    run_command(cargo_args, &default_envs, extra_args, None);
}

fn run_command(
    cargo_args: &[&str],
    default_envs: &HashMap<String, String>,
    extra_args: &[String],
    explain: Option<&str>,
) {
    let mut cmd = Command::new("cargo");
    cmd.args(cargo_args);
    cmd.args(extra_args);

    for (key, value) in default_envs {
        cmd.env(key, value);
    }

    println!(
        "> Running: cargo {} {}",
        cargo_args.join(" "),
        extra_args.join(" ")
    );
    println!("> With envs: {default_envs:?}");

    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to run cargo (explain: {explain:?}): {e}");
            exit(1);
        }
    };
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "Usage: xtask {{
    build               build in debug mode
    run                 runs executable
    build:release       build in release mode
    run:release         runs executable in release mode
    build:test          build and runs tests
    build:qnx_x86_64    build for QNX7.1 target: x86_64-pc-nto-qnx710
    build:qnx_arm       build for QNX7.1 target: aarch64-pc-nto-qnx710
    clippy              runs clippy
    fmt                 runs rustfmt
    fmt:check           checks formatting without modifying files
    miri                runs Miri UB checker (excludes integration_tests, xtask)
    coverage            runs cargo-llvm-cov (excludes xtask)
    check               runs fundamental checks, good to run before push
    check_lic           runs source code license check

    [ENV_VAR=value ...] [-- cargo args...]"
    );
    exit(1);
}

const REQUIRED_HEADER: &str = r"/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 */
//";

fn check_license_header() {
    let project_dir = match std::env::current_dir() {
        Ok(d) => d.join("src"),
        Err(e) => {
            eprintln!("Failed to get current directory: {e}");
            exit(1);
        }
    };
    let mut missing_header_files = Vec::new();

    visit_dirs(&project_dir, &mut missing_header_files);

    if missing_header_files.is_empty() {
        println!("All files have the required license header.");
    } else {
        println!("The following files are missing the required license header:");
        println!("\n{REQUIRED_HEADER}\n");
        for file in missing_header_files {
            println!("{}", file.display());
        }

        std::process::exit(-1);
    }
}

fn visit_dirs(dir: &Path, missing_header_files: &mut Vec<std::path::PathBuf>) {
    if dir.is_dir() {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to read directory {}: {e}", dir.display());
                exit(1);
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("Failed to get directory entry: {e}");
                    exit(1);
                }
            };
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, missing_header_files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                check_file(&path, missing_header_files);
            }
        }
    }
}

fn check_file(file_path: &Path, missing_header_files: &mut Vec<std::path::PathBuf>) {
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read file {}: {e}", file_path.display());
            exit(1);
        }
    };
    if !content.starts_with(REQUIRED_HEADER) {
        missing_header_files.push(file_path.to_path_buf());
    }
}
