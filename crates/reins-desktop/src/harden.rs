//! Process hardening: other processes of the same user cannot attach to the daemon (ptrace) or read its memory
//! (`/proc/<pid>/mem`), where the GitHub credentials live, and it leaves no core dumps.

/// Makes the process non-dumpable on Linux; elsewhere does nothing.
pub fn harden() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
            .map_err(|e| format!("cannot make the process non-dumpable: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(target_os = "linux")]
    fn the_process_becomes_non_dumpable() {
        super::harden().unwrap();
        assert_eq!(rustix::process::dumpable_behavior().unwrap(), rustix::process::DumpableBehavior::NotDumpable);
    }
}
