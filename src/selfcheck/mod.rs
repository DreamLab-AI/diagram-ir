//! The packaged self-check: the accessible-SVG contract, the single-file safety
//! rules, and the structural motion contract. A distilled subset of the
//! repository gates, which remain the authority for repository contributions.

pub mod checks;
pub mod html;
pub mod refs;

use std::path::PathBuf;

/// The canonical motion controller, compiled into the crate from
/// `assets/template-motion.html` so a standalone binary needs no skill checkout.
/// An installed diagram-design skill can still override it (see
/// [`resolve_motion_template`]); the two are expected to be byte-identical.
pub const BUNDLED_MOTION_TEMPLATE: &str = include_str!("../../assets/template-motion.html");

/// The pseudo-path [`resolve_motion_template`] returns when no template exists
/// on disk. [`checks::canonical_controller`] recognises it and reads
/// [`BUNDLED_MOTION_TEMPLATE`] instead of the filesystem.
pub const BUNDLED_MOTION_TEMPLATE_PATH: &str = "<bundled>/assets/template-motion.html";

/// Where the canonical motion controller lives. The binary no longer sits
/// inside the skill, so the template is located by flag, then environment, then
/// the installed and in-repo locations, and finally the copy compiled into this
/// crate ([`BUNDLED_MOTION_TEMPLATE`]).
pub fn resolve_motion_template(explicit: Option<&str>) -> PathBuf {
    if let Some(explicit) = explicit {
        return PathBuf::from(explicit);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(skill_dir) = std::env::var("DIAGRAM_DESIGN_SKILL_DIR") {
        if !skill_dir.is_empty() {
            candidates.push(
                PathBuf::from(skill_dir)
                    .join("assets")
                    .join("template-motion.html"),
            );
        }
    }
    candidates.push(PathBuf::from(
        "/opt/agentbox/skills/diagram-design/assets/template-motion.html",
    ));
    candidates.push(PathBuf::from(
        "./skills/diagram-design/assets/template-motion.html",
    ));
    for candidate in &candidates {
        if candidate.is_file() {
            return candidate.clone();
        }
    }
    PathBuf::from(BUNDLED_MOTION_TEMPLATE_PATH)
}
