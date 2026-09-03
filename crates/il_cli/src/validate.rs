//! `il_cli validate <roots...>` (T1-027, REQ-TEST-005): loads the mods under
//! the given roots through the full pipeline and prints every diagnostic.

use std::io::Write;
use std::path::PathBuf;

use il_data::{Diagnostics, ModSet, discover, pipeline::load_report};

#[derive(Clone, Debug)]
pub struct ValidateOptions {
    /// Mod roots; the first is the game (`game/` by default).
    pub roots: Vec<PathBuf>,
    /// Treat warnings as failures.
    pub deny_warnings: bool,
    /// Also print the load order, both hashes and per-registry counts.
    pub verbose: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidateReport {
    pub errors: usize,
    pub warnings: usize,
    /// Mods in resolved load order.
    pub mods: Vec<String>,
}

impl ValidateReport {
    /// Exit status: clean, or only warnings when they are allowed.
    pub fn ok(&self, deny_warnings: bool) -> bool {
        self.errors == 0 && (!deny_warnings || self.warnings == 0)
    }
}

fn print_all(diags: &Diagnostics, out: &mut dyn Write) -> std::io::Result<()> {
    for d in diags.warnings() {
        writeln!(out, "{d}")?;
    }
    for d in diags.errors() {
        writeln!(out, "{d}")?;
    }
    Ok(())
}

/// Runs discovery, load-order resolution and the load pipeline, printing
/// warnings, then errors, then a summary line.
pub fn validate(opts: &ValidateOptions, out: &mut dyn Write) -> anyhow::Result<ValidateReport> {
    let found = match discover(&opts.roots) {
        Ok(f) => f,
        Err(diags) => {
            print_all(&diags, out)?;
            let report = ValidateReport {
                errors: diags.errors().count(),
                warnings: diags.warnings().count(),
                mods: Vec::new(),
            };
            writeln!(
                out,
                "{} errors, {} warnings (manifests)",
                report.errors, report.warnings
            )?;
            return Ok(report);
        }
    };
    let set = match ModSet::all(&found) {
        Ok(set) => set,
        Err(errors) => {
            for e in &errors {
                writeln!(out, "{e}")?;
            }
            writeln!(out, "{} errors, 0 warnings (load order)", errors.len())?;
            return Ok(ValidateReport {
                errors: errors.len(),
                warnings: 0,
                mods: Vec::new(),
            });
        }
    };
    let mods: Vec<String> = set.mods.iter().map(|m| m.manifest.id.clone()).collect();
    for w in &set.warnings {
        writeln!(out, "warning: {w}")?;
    }
    let report = load_report(&set);
    print_all(&report.diagnostics, out)?;
    let errors = report.diagnostics.errors().count();
    let warnings = report.diagnostics.warnings().count() + set.warnings.len();
    writeln!(
        out,
        "{errors} errors, {warnings} warnings in {} mod{} (order: {})",
        mods.len(),
        if mods.len() == 1 { "" } else { "s" },
        mods.join(", ")
    )?;
    if opts.verbose
        && let Some(regs) = &report.registries
    {
        writeln!(out, "mod list hash {:016x}", regs.mod_list_hash)?;
        writeln!(out, "content hash {:016x}", regs.content_registry_hash)?;
        writeln!(
            out,
            "units {} · formations {} · group formations {} · factions {} · zones {} · maps {} · sprite sets {} · locale languages {}",
            regs.units.len(),
            regs.formations.len(),
            regs.group_formations.len(),
            regs.factions.len(),
            regs.zones.len(),
            regs.maps.len(),
            regs.sprite_sets.len(),
            regs.locale.languages().count()
        )?;
    }
    Ok(ValidateReport {
        errors,
        warnings,
        mods,
    })
}
