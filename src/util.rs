//! Utility functions.

use std::collections::{HashMap, HashSet};
use std::convert::TryInto;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::Context;
use log::warn;

use crate::config::get_main_branch_name;

/// Convert a `git2::Error` into an `anyhow::Error` with an auto-generated message.
pub fn wrap_git_error(error: git2::Error) -> anyhow::Error {
    anyhow::anyhow!("Git error {:?}: {}", error.code(), error.message())
}

/// Get the OID for the repository's `HEAD` reference.
///
/// Args:
/// * `repo`: The Git repository.
///
/// Returns: The OID for the repository's `HEAD` reference.
pub fn get_head_oid(repo: &git2::Repository) -> anyhow::Result<Option<git2::Oid>> {
    let head_ref = match repo.head() {
        Ok(head_ref) => Ok(head_ref),
        Err(err)
            if err.code() == git2::ErrorCode::NotFound
                || err.code() == git2::ErrorCode::UnbornBranch =>
        {
            return Ok(None)
        }
        Err(err) => Err(err),
    }?;
    let head_commit = head_ref.peel_to_commit()?;
    Ok(Some(head_commit.id()))
}

/// Get the OID corresponding to the main branch.
///
/// Args:
/// * `repo`: The Git repository.
///
/// Returns: The OID corresponding to the main branch.
pub fn get_main_branch_oid(repo: &git2::Repository) -> anyhow::Result<git2::Oid> {
    let main_branch_name = get_main_branch_name(&repo)?;
    let branch = repo
        .find_branch(&main_branch_name, git2::BranchType::Local)
        .or_else(|_| repo.find_branch(&main_branch_name, git2::BranchType::Remote))?;
    let commit = branch.get().peel_to_commit()?;
    Ok(commit.id())
}

/// Get a mapping from OID to the names of branches which point to that OID.
pub fn get_branch_oid_to_names(
    repo: &git2::Repository,
) -> anyhow::Result<HashMap<git2::Oid, HashSet<String>>> {
    let branches = repo
        .branches(Some(git2::BranchType::Local))
        .with_context(|| "Reading branches")?;

    let mut result = HashMap::new();
    for branch_info in branches {
        let branch_info = branch_info.with_context(|| "Iterating over branches")?;
        match branch_info {
            (branch, git2::BranchType::Remote) => anyhow::bail!(
                "Unexpectedly got a remote branch in local branch iterator: {:?}",
                branch.name()
            ),
            (branch, git2::BranchType::Local) => {
                let reference = branch.into_reference();
                match reference.name() {
                    None => warn!(
                        "Could not decode branch name, skipping: {:?}",
                        reference.name_bytes()
                    ),
                    Some(reference_name) => {
                        let commit = reference.peel_to_commit().with_context(|| {
                            format!("Peeling branch into commit: {}", reference_name)
                        })?;
                        let branch_oid = commit.id();
                        result
                            .entry(branch_oid)
                            .or_insert_with(HashSet::new)
                            .insert(reference_name.to_owned());
                    }
                }
            }
        }
    }
    Ok(result)
}

/// Get the git repository associated with the current directory.
pub fn get_repo() -> anyhow::Result<git2::Repository> {
    let path = std::env::current_dir().with_context(|| "Getting working directory")?;
    let repository = git2::Repository::discover(path).map_err(wrap_git_error)?;
    Ok(repository)
}

/// Get the connection to the SQLite database for this repository.
pub fn get_db_conn(repo: &git2::Repository) -> anyhow::Result<rusqlite::Connection> {
    let dir = repo.path().join("branchless");
    std::fs::create_dir_all(&dir).with_context(|| "Creating .git/branchless dir")?;
    let path = dir.join("db.sqlite3");
    let conn = rusqlite::Connection::open(&path)
        .with_context(|| format!("Opening database connection at {:?}", &path))?;
    Ok(conn)
}

/// Path to the `git` executable on disk to be executed.
#[derive(Debug)]
pub struct GitExecutable<'path>(pub &'path Path);

/// Run Git in a subprocess, and inform the user.
///
/// This is suitable for commands which affect the working copy or should run
/// hooks. We don't want our process to be responsible for that.
///
/// Args:
/// * `out`: The output stream to write to.
/// * `err`: The error stream to write to.
/// * `git_executable`: The path to the `git` executable on disk.
/// * `args`: The list of arguments to pass to Git. Should not include the Git
/// executable itself.
///
/// Returns: The exit code of Git (non-zero signifies error).
pub fn run_git<Out: std::io::Write, S: AsRef<str> + std::fmt::Debug>(
    out: &mut Out,
    err: &mut Out,
    git_executable: &GitExecutable,
    args: &[S],
) -> anyhow::Result<isize> {
    let GitExecutable(git_executable) = git_executable;
    writeln!(
        out,
        "branchless: {} {}",
        git_executable.to_string_lossy(),
        args.iter()
            .map(|arg| arg.as_ref())
            .collect::<Vec<_>>()
            .join(" ")
    )?;
    out.flush()?;
    err.flush()?;

    let mut child = Command::new(git_executable)
        .args(args.iter().map(|arg| arg.as_ref()))
        .spawn()
        .with_context(|| format!("Spawning Git subprocess: {:?} {:?}", git_executable, args))?;
    let result = child.wait().with_context(|| {
        format!(
            "Waiting for Git subprocess to complete: {:?} {:?}",
            git_executable, args
        )
    })?;
    // On Unix, if the child process was terminated by a signal, we need to call
    // some Unix-specific functions to access the signal that terminated it. For
    // simplicity, just return `1` in those cases.
    let exit_code = result.code().unwrap_or(1);
    let exit_code = exit_code
        .try_into()
        .with_context(|| format!("Converting exit code {} from i32 to isize", exit_code))?;
    Ok(exit_code)
}

/// Run Git silently (don't display output to the user).
///
/// Whenever possible, use `git2`'s bindings to Git instead, as they're
/// considerably more lightweight and reliable.
///
/// Args:
/// * `repo`: The Git repository.
/// * `git_executable`: The path to the `git` executable on disk.
/// * `args`: The command-line args to pass to Git.
///
/// Returns: the stdout of the Git invocation.
pub fn run_git_silent<S: AsRef<str> + std::fmt::Debug>(
    repo: &git2::Repository,
    git_executable: &GitExecutable,
    args: &[S],
) -> anyhow::Result<String> {
    let GitExecutable(git_executable) = git_executable;

    // Technically speaking, we should be able to work with non-UTF-8 repository
    // paths. Need to make the typechecker accept it.
    let repo_path = repo.path();
    let repo_path = repo_path.to_str().ok_or_else(|| {
        anyhow::anyhow!(
            "Path to Git repo could not be converted to UTF-8 string: {:?}",
            repo_path
        )
    })?;

    let args = {
        let mut result = vec!["-C", repo_path];
        result.extend(args.iter().map(|arg| arg.as_ref()));
        result
    };
    let result = Command::new(git_executable)
        .args(&args)
        .output()
        .with_context(|| format!("Spawning Git subprocess: {:?} {:?}", git_executable, args))?;
    let result = String::from_utf8(result.stdout).with_context(|| {
        format!(
            "Decoding stdout from Git subprocess: {:?} {:?}",
            git_executable, args
        )
    })?;
    Ok(result)
}

/// The parsed version of Git.
#[derive(Debug, PartialEq, PartialOrd, Eq)]
pub struct GitVersion(pub isize, pub isize, pub isize);

impl FromStr for GitVersion {
    type Err = anyhow::Error;

    fn from_str(output: &str) -> anyhow::Result<GitVersion> {
        let output = output.trim();
        let words = output.split(' ').collect::<Vec<&str>>();
        let version_str = match &words.as_slice() {
            [_git, _version, version_str, ..] => version_str,
            _ => anyhow::bail!("Could not parse Git version output: {}", output),
        };
        match version_str.split('.').collect::<Vec<&str>>().as_slice() {
            [major, minor, patch, ..] => {
                let major = major.parse()?;
                let minor = minor.parse()?;
                let patch = patch.parse()?;
                Ok(GitVersion(major, minor, patch))
            }
            _ => anyhow::bail!("Could not parse Git version string: {}", version_str),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_version_output() {
        assert_eq!(
            "git version 12.34.56".parse::<GitVersion>().unwrap(),
            GitVersion(12, 34, 56)
        );
        assert_eq!(
            "git version 12.34.56\n".parse::<GitVersion>().unwrap(),
            GitVersion(12, 34, 56)
        );
        assert_eq!(
            "git version 12.34.56.78.abcdef"
                .parse::<GitVersion>()
                .unwrap(),
            GitVersion(12, 34, 56)
        );
    }
}